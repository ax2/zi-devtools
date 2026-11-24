"""Freeze the independent directory comparison plugin candidate."""

from package_disk_inspector_plugin import integer, obj, string
from package_duplicate_files_plugin import candidate_arguments, freeze_candidate
from verify_directory_compare_plugin import CAPABILITY, MAX_REPORT, PERMISSION, PLUGIN, PROFILE, STATES, cases

VERSION = "0.1.0"
TITLE = "目录内容对比"
SCOPE = "按本次确认的相对路径对比两个目录的类型、逻辑大小和摘要。清点不足时不推断单侧缺失。"


def report_schema():
    boolean = {"type": "boolean"}
    file = obj(kind={"type": "string", "const": "file"}, logicalBytes=string(20), sha256=string(64))
    file["required"] = ["kind", "logicalBytes"]
    directory = obj(kind={"type": "string", "const": "directory"})
    observed = {"type": "object", "oneOf": [file, directory]}
    row = obj(path=string(2048), difference={"type": "string", "enum": list(STATES)},
              left=observed, right=observed)
    row["required"] = ["path", "difference"]
    count = integer(256)
    success = obj(
        ok={"type": "boolean", "const": True},
        schemaVersion={"type": "integer", "const": 1},
        material={"type": "string", "const": "directory-compare-report"},
        coverageComplete=boolean, partial=boolean,
        limitReasons={"type": "array", "maxItems": 16, "items": string(32)},
        totalPaths=count, sameFiles=count, sameDirectories=count, unassessedOneSide=count,
        differenceCount=count, differenceCounts=obj(**{state: count for state in STATES}),
        rows={"type": "array", "maxItems": 256, "items": row},
        projectionTruncated=boolean,
    )
    error = obj(ok={"type": "boolean", "const": False}, error=obj(code=string(40), message=string(256)))
    return {"type": "object", "oneOf": [success, error]}


def manifest():
    return dict(
        schemaVersion=2, id=PLUGIN, name="ZiDevTools Directory Compare", displayName=TITLE,
        description=SCOPE, version=VERSION, publisher="zicode", license="MIT",
        engines={"zicode": ">=0.7.2 <1.0.0", "pluginApi": "^1.0.0"},
        requiredProfiles={"resource.input.types.v1": "=1.0.0-preview.1",
                          "output.schema.v1": "=1.0.0-rc.1",
                          "tool.directory.v1": "=1.0.0-preview.1"},
        activationEvents=["onCapability:" + CAPABILITY],
        runtime={"wasi": "runtime/tools.wasm", "resourceInputProfile": "resource.wasi.input.draft2"},
        contributes=dict(activities=[], views=[], commands=[], piTools=[], capabilities=[dict(
            id=CAPABILITY, title=TITLE, description=SCOPE,
            resourceInputMimeTypes=["text/plain"], inputSchema=obj(), outputSchema=report_schema(),
            maxResultBytes=MAX_REPORT,
            directory=dict(categoryId="files", aliases=["文件夹对比", "比较文件夹", "directory compare"],
                           description=SCOPE),
        )]),
        permissions=dict(required=[
            dict(name="resource.read", reason="读取本次确认的目录对比快照。"),
            dict(name=PERMISSION, reason="明确确认后由 Studio 比较两个目录中的相对条目。"),
        ], optional=[]),
    )


def main():
    parser, args = candidate_arguments(__doc__)
    freeze_candidate(parser, args, slug="directory-compare", source_tool="file-compare",
                     profile=PROFILE, manifest=manifest(), vectors=cases(),
                     fixture_generator="scripts/verify_directory_compare_plugin.py")


if __name__ == "__main__":
    main()
