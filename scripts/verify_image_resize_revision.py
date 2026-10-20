"""Execute the frozen image runtime against small format and parameter fixtures; no Studio runtime."""
import argparse
import hashlib
from io import BytesIO
import json
from pathlib import Path
import subprocess
import tempfile

import wasmtime
import wasmtime._ffi as ffi
from PIL import Image
from jsonschema import Draft202012Validator

import benchmark_studio_image_resource as runner

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--wasm', type=Path, default=Path('C:/Users/ax2/AppData/Local/ZiDevTools/zi-devtools/wasm32-wasip1/plugin/zi-image-wasi.wasm'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--native', type=Path, required=True, help='Prebuilt native image_core_benchmark executable')
    args = parser.parse_args()
    module = args.wasm.read_bytes()
    schema = json.loads((ROOT/'plugins/image-resize/schemas/resize-input.schema.json').read_bytes())
    output_schema = json.loads((ROOT/'plugins/image-resize/schemas/resize-output.schema.json').read_bytes())
    validator = Draft202012Validator(schema)
    rows = []
    with tempfile.TemporaryDirectory(prefix='zi-image-contract-') as temporary:
        runner.BENCH = Path(temporary)
        formats = {'png': 'PNG', 'jpeg': 'JPEG', 'webp': 'WEBP'}
        inputs = {}
        for name, fmt in formats.items():
            image = Image.new('RGB' if name == 'jpeg' else 'RGBA', (32,16), (21,104,187) if name == 'jpeg' else (21,104,187,73))
            stream = BytesIO()
            image.save(stream, format=fmt, lossless=True)
            inputs[name] = stream.getvalue()

        def execute(case, operation, mime, data):
            request = runner.inner_request(operation, mime, data)
            result = runner.run_direct_wasmtime(wasmtime, ffi, module, request, mime, data, case, runner.IMAGE_RESIZE_FUEL_BUDGET)
            if result['error'] or result['stderr']:
                raise RuntimeError(f'{case} failed: {result}')
            # Keep only content evidence; temporary output paths expire after verification.
            for output in result.get('outputResources', []):
                output.pop('path', None)
            result['inputSchemaValid'] = validator.is_valid(operation)
            rows.append(result)
            return result

        for source, data in inputs.items():
            for target in formats:
                operation = {'maxWidth':16,'format':target,'quality':82}
                result = execute(f'{source}-to-{target}', operation, 'image/'+source, data)
                assert result['inputSchemaValid'] and result['result']['ok']
                Draft202012Validator(output_schema).validate(result['result'])
                assert len(result['outputResources']) == 1
                output = result['outputResources'][0]
                assert output['mimeType'] == 'image/'+target and output['decodedOutput']['size'] == [16,8]
                if source != 'jpeg' and target != 'jpeg':
                    assert output['decodedOutput']['alphaSample'] == 73
                input_path = Path(temporary)/f'native-input.{source}'
                output_path = Path(temporary)/f'native-output.{target}'
                input_path.write_bytes(data)
                native = subprocess.run([str(args.native),str(input_path),str(output_path),'16',target,'82'],
                                        capture_output=True,text=True,check=True,timeout=30)
                metadata = json.loads(native.stdout)
                native_sha = hashlib.sha256(output_path.read_bytes()).hexdigest()
                assert native_sha == output['sha256']
                assert metadata['width'] == 16 and metadata['height'] == 8
                result['nativeComparison'] = {'byteIdentical':True,'sha256':native_sha,'metadata':metadata}
                legacy = execute(f'legacy-{source}-to-{target}', {'maxWidth':'0016','format':target,'quality':'82'}, 'image/'+source, data)
                assert not legacy['inputSchemaValid'] and legacy['result'] == result['result']
                assert legacy['outputResources'][0]['sha256'] == output['sha256']
        for field, value in [('maxWidth','abc'),('maxWidth',4097),('maxWidth','1\n'),('quality',96)]:
            operation = {'maxWidth':16,'format':'webp','quality':82,field:value}
            result = execute(f'invalid-{field}-{len(rows)}', operation, 'image/png', inputs['png'])
            assert not result['inputSchemaValid'] and result['result']['error']['code'] == 'INVALID_PARAMETERS'
            assert result['inputReadCalls'] == 0 and result['outputCalls'] == 0 and not result.get('outputResources')
        result = execute('damaged-png', {'maxWidth':16,'format':'png','quality':82}, 'image/png', b'not a PNG')
        assert not result['result']['ok'] and not result.get('outputResources') and result['outputCalls'] == 0
    report = {'wasmSha256': hashlib.sha256(module).hexdigest(),
              'nativeSha256':hashlib.sha256(args.native.read_bytes()).hexdigest(),
              'inputSchemaSha256': hashlib.sha256((ROOT/'plugins/image-resize/schemas/resize-input.schema.json').read_bytes()).hexdigest(),
              'cases':rows,'passed':True,
              'limitations':['Direct WASI only; no Studio runtime or resource store',
                             'Small 32x16 fixtures; no claim of a new 16MP or concurrency benchmark',
                             '30B fuel / 256MiB guest limit; no production 10-second lifecycle deadline test']}
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(f'PASS {len(rows)} actual image WASI cases; output metadata/decode, alpha and zero-output errors verified')


if __name__ == '__main__':
    main()
