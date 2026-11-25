//! Inspect one read-only, self-contained SQLite file and optionally export a complete table page.
use rusqlite::{Connection, DatabaseName, ffi, serialize::OwnedData};
use serde::Deserialize;
use serde_json::{Value, json};
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write as _;
use std::{collections::BTreeSet, fmt::Write as _, io::Read, ptr::NonNull};
use zi_table_core::sqlite_read::{self, PageData, PageLimits};

const PLUGIN: &str = "com.zicode.devtools.sqlite-browser";
const CAPABILITY: &str = "devtools.sqlite.snapshot.browse";
const INPUT: &str = "resource.wasi.input.draft2";
const OUTPUT: &str = "resource.wasi.output.draft1";
const VERSION: &str = "1.0.0-rc.1";
const MAX_INPUT: usize = 128 * 1024;
const MAX_RESULT: usize = 48 * 1024;
const MAX_CSV: usize = 24 * 1024;
const MAX_SCHEMA: usize = 64;
const LIMITS: PageLimits = PageLimits {
    page_size: 16,
    max_pages: 16,
    max_columns: 16,
    max_cell_bytes: 512,
    max_page_bytes: 6 * 1024,
};

type Failure = (&'static str, &'static str);
const REQUEST: Failure = ("INVALID_REQUEST", "SQLite 浏览请求无效。");
const RESOURCE: Failure = ("INVALID_RESOURCE", "请选择并单独确认一份 SQLite 文件。");
const READ: Failure = ("RESOURCE_READ_FAILED", "无法读取已确认的 SQLite 文件。");
const DATABASE: Failure = ("INVALID_DATABASE", "无法以只读方式读取此 SQLite 文件。");
const TABLE: Failure = (
    "UNSUPPORTED_TABLE",
    "请选择目录中的普通数据表。视图、虚拟表和生成列暂不支持查看。",
);
const QUERY: Failure = (
    "PAGE_READ_FAILED",
    "无法读取此数据页。可查看最多 16 列的普通表。",
);
const RESULT: Failure = ("RESULT_TOO_LARGE", "本页的列名或数据超过预览范围。");
const CSV: Failure = ("CSV_WRITE_FAILED", "无法生成当前页的 CSV 文件。");

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    plugin_id: String,
    capability_id: String,
    scene_id: String,
    command_id: Value,
    resource_input_profile: String,
    resource_output_profile: Option<String>,
    resources: Vec<File>,
    input: Options,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Options {
    table_name: Option<String>,
    page: Option<usize>,
    #[serde(default)]
    export_csv: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct File {
    index: usize,
    resource_id: String,
    content_version: String,
    mime_type: String,
    byte_length: usize,
}

fn open_snapshot(bytes: &[u8]) -> Result<Connection, Failure> {
    if bytes.len() < 512 || !bytes.starts_with(b"SQLite format 3\0") {
        return Err(DATABASE);
    }
    if bytes[18] == 2 || bytes[19] == 2 {
        return Err((
            "WAL_SNAPSHOT",
            "请选择完整的 SQLite 独立副本。当前文件依赖 WAL 或其他辅助文件。",
        ));
    }
    if bytes[18] != 1 || bytes[19] != 1 {
        return Err(DATABASE);
    }
    let encoded_size = u16::from_be_bytes([bytes[16], bytes[17]]);
    let page_size = if encoded_size == 1 {
        65_536
    } else {
        usize::from(encoded_size)
    };
    if !(512..=65_536).contains(&page_size)
        || !page_size.is_power_of_two()
        || !bytes.len().is_multiple_of(page_size)
    {
        return Err(DATABASE);
    }
    let encoding = u32::from_be_bytes([bytes[56], bytes[57], bytes[58], bytes[59]]);
    if !matches!(encoding, 0 | 1) {
        return Err((
            "UNSUPPORTED_ENCODING",
            "请选择使用 UTF-8 编码的 SQLite 数据库。",
        ));
    }
    let declared_pages = u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
    if bytes[24..28] == bytes[92..96]
        && declared_pages != 0
        && declared_pages as usize != bytes.len() / page_size
    {
        return Err(DATABASE);
    }
    let mut connection = Connection::open_in_memory().map_err(|_| DATABASE)?;
    connection
        .pragma_update(None, "trusted_schema", false)
        .map_err(|_| DATABASE)?;
    let pointer = NonNull::new(unsafe { ffi::sqlite3_malloc64(bytes.len() as u64) }.cast::<u8>())
        .ok_or(DATABASE)?;
    // SQLite owns the copied allocation. The original snapshot remains read-only.
    let owned = unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.as_ptr(), bytes.len());
        OwnedData::from_raw_nonnull(pointer, bytes.len())
    };
    connection
        .deserialize(DatabaseName::Main, owned, true)
        .map_err(|_| DATABASE)?;
    connection
        .pragma_update(None, "query_only", true)
        .map_err(|_| DATABASE)?;
    Ok(connection)
}

fn quote_display(text: &str) -> String {
    serde_json::to_string(text).expect("serializing a string cannot fail")
}

fn list_entries(db: &Connection) -> Result<Vec<(String, String)>, Failure> {
    // Bound the catalog before table_list computes metadata for each view.
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM (
            SELECT 1 FROM sqlite_schema WHERE type IN ('table','view')
            AND substr(name,1,7) <> 'sqlite_' COLLATE NOCASE LIMIT ?1
        )",
            [MAX_SCHEMA as i64 + 1],
            |row| row.get(0),
        )
        .map_err(|_| DATABASE)?;
    if count > MAX_SCHEMA as i64 {
        return Err((
            "SCHEMA_LIMIT",
            "单份 SQLite 文件最多检查 64 个表与视图定义。",
        ));
    }
    let mut query = db
        .prepare(
            "SELECT name,type FROM pragma_table_list WHERE schema='main'
         AND substr(name,1,7) <> 'sqlite_' COLLATE NOCASE
         ORDER BY type,name",
        )
        .map_err(|_| DATABASE)?;
    let mut entries = query
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
        .map_err(|_| DATABASE)?;
    // A build without a virtual-table module may see its auxiliary tables as ordinary tables.
    let virtual_names = entries
        .iter()
        .filter(|(_, kind)| kind == "virtual")
        .map(|(name, _)| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    entries.retain(|(name, kind)| {
        kind != "shadow"
            && (kind != "table" || {
                let name = name.to_ascii_lowercase();
                !name
                    .match_indices('_')
                    .any(|(end, _)| virtual_names.contains(&name[..end]))
            })
    });
    Ok(entries)
}

fn catalog(entries: &[(String, String)], page: usize) -> Value {
    let start = ((page - 1) * LIMITS.page_size).min(entries.len());
    let end = (start + LIMITS.page_size).min(entries.len());
    let more = end < entries.len();
    let entries = &entries[start..end];
    let mut text = format!("SQLite 目录 · 第 {page} 页 · {} 项", entries.len());
    for (name, kind) in entries {
        let kind = match kind.as_str() {
            "table" => "普通表",
            "view" => "视图",
            _ => "虚拟表",
        };
        let _ = write!(text, "\n{kind} · {}", quote_display(name));
    }
    json!({"kind":"catalog","page":page,"rowCount":entries.len(),"columnCount":0,
           "hasNext":more && page<LIMITS.max_pages, "truncatedCells":0,"csvBytes":null,"text":text})
}

fn read_table(
    db: &Connection,
    entries: &[(String, String)],
    name: &str,
    page: usize,
) -> Result<PageData, Failure> {
    if name.len() > 512 || name.contains('\0') {
        return Err(TABLE);
    }
    if !entries
        .iter()
        .any(|(entry, kind)| entry == name && kind == "table")
    {
        return Err(TABLE);
    }
    let generated: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_xinfo(?1) WHERE hidden <> 0)",
            [name],
            |row| row.get(0),
        )
        .map_err(|_| QUERY)?;
    if generated {
        return Err(TABLE);
    }
    sqlite_read::load_page(db, name, page - 1, LIMITS, || Ok(())).map_err(|_| QUERY)
}

fn render_page(data: &PageData, page: usize) -> String {
    let mut text = format!(
        "{} · 第 {page} 页 · {} 行",
        quote_display(&data.table),
        data.rows.len()
    );
    let names = data
        .columns
        .iter()
        .map(|column| quote_display(&column.name))
        .collect::<Vec<_>>();
    let _ = write!(text, "\n列 · {}", names.join(" | "));
    for (index, row) in data.rows.iter().enumerate() {
        let cells = row
            .iter()
            .map(|cell| {
                if cell.kind == "文本" {
                    quote_display(&cell.display)
                } else {
                    cell.display.clone()
                }
            })
            .collect::<Vec<_>>();
        let _ = write!(
            text,
            "\n{} · {}",
            (page - 1) * LIMITS.page_size + index + 1,
            cells.join(" | ")
        );
    }
    if data.truncated_cells > 0 {
        let _ = write!(
            text,
            "\n此页有 {} 个值仅显示部分内容。",
            data.truncated_cells
        );
    }
    if data.has_next {
        text.push_str("\n还有下一页。");
    }
    text
}

fn execute() -> Result<Vec<u8>, Failure> {
    let mut envelope = Vec::new();
    std::io::stdin()
        .take(16 * 1024 + 1)
        .read_to_end(&mut envelope)
        .map_err(|_| REQUEST)?;
    if envelope.len() > 16 * 1024 {
        return Err(REQUEST);
    }
    let request: Request = serde_json::from_slice(&envelope).map_err(|_| REQUEST)?;
    if request.plugin_id != PLUGIN
        || request.capability_id != CAPABILITY
        || request.resource_input_profile != INPUT
        || request.resource_output_profile.as_deref() != Some(OUTPUT)
        || !request.command_id.is_null()
        || request.scene_id.is_empty()
        || request.scene_id.len() > 128
    {
        return Err(REQUEST);
    }
    let [file] = request.resources.as_slice() else {
        return Err(RESOURCE);
    };
    if file.index != 0
        || file.resource_id.is_empty()
        || file.content_version.len() != 64
        || !file
            .content_version
            .bytes()
            .all(|value| value.is_ascii_hexdigit())
    {
        return Err(RESOURCE);
    }
    if !matches!(
        file.mime_type.as_str(),
        "application/octet-stream" | "application/x-zicode-sqlite"
    ) {
        return Err(("UNSUPPORTED_MIME", "请选择一份 SQLite 数据库文件。"));
    }
    if file.byte_length > MAX_INPUT {
        return Err(("INPUT_TOO_LARGE", "每份 SQLite 文件最多 128 KiB。"));
    }
    let page = request.input.page.unwrap_or(1);
    if !(1..=LIMITS.max_pages).contains(&page) {
        return Err(("PAGE_LIMIT", "最多查看前 16 页。"));
    }
    let bytes = read(file.byte_length)?;
    let db = open_snapshot(&bytes)?;
    let entries = list_entries(&db)?;
    let name = request
        .input
        .table_name
        .as_deref()
        .filter(|name| !name.is_empty());
    let mut csv = Vec::new();
    let result = if let Some(table) = name {
        let data = read_table(&db, &entries, table, page)?;
        if request.input.export_csv {
            if data.rows.is_empty()
                || data
                    .rows
                    .iter()
                    .flatten()
                    .any(|cell| cell.truncated || cell.value.is_none())
            {
                return Err((
                    "CSV_PAGE_UNAVAILABLE",
                    "此页为空，或含无法完整写入 CSV 的值。可关闭导出后继续预览。",
                ));
            }
            sqlite_read::write_csv(&data, &mut csv).map_err(|_| CSV)?;
            if csv.len() > MAX_CSV {
                return Err(("CSV_TOO_LARGE", "当前页 CSV 超过 24 KiB。"));
            }
        }
        let mut text = render_page(&data, page);
        if !csv.is_empty() {
            text.push_str("\n已生成当前页 CSV，请确认后另存。");
        }
        json!({"kind":"table","page":page,"rowCount":data.rows.len(),"columnCount":data.columns.len(),
               "hasNext":data.has_next,"truncatedCells":data.truncated_cells,
               "csvBytes":request.input.export_csv.then_some(csv.len()),"text":text})
    } else {
        if request.input.export_csv {
            return Err(("CSV_NEEDS_TABLE", "请指定一个普通表再导出当前页。"));
        }
        catalog(&entries, page)
    };
    let output = serde_json::to_vec(
        &json!({"contractVersion":VERSION,"ok":true,"data":result,"error":null}),
    )
    .map_err(|_| RESULT)?;
    if output.len() > MAX_RESULT {
        return Err(RESULT);
    }
    if !csv.is_empty() {
        publish(&csv)?;
    }
    Ok(output)
}

#[cfg(target_arch = "wasm32")]
fn read(length: usize) -> Result<Vec<u8>, Failure> {
    zi_resource_input_wasi::read_window(0, length, 0, length).map_err(|_| READ)
}

#[cfg(not(target_arch = "wasm32"))]
fn read(length: usize) -> Result<Vec<u8>, Failure> {
    let path = std::env::args_os().nth(1).ok_or(READ)?;
    let source = std::fs::File::open(path).map_err(|_| READ)?;
    let mut bytes = Vec::new();
    source
        .take((length + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| READ)?;
    if bytes.len() != length {
        return Err(READ);
    }
    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
fn publish(bytes: &[u8]) -> Result<(), Failure> {
    const CSV_MIME: i32 = 23;
    #[link(wasm_import_module = "zicode_resource_output_draft1")]
    unsafe extern "C" {
        #[link_name = "create"]
        fn output_create(mime: i32, length: i64) -> i32;
        #[link_name = "write"]
        fn output_write(slot: i32, pointer: *const u8, length: usize) -> i32;
        #[link_name = "finish"]
        fn output_finish(slot: i32) -> i32;
    }
    let slot = unsafe { output_create(CSV_MIME, bytes.len() as i64) };
    if slot < 0
        || unsafe { output_write(slot, bytes.as_ptr(), bytes.len()) } != bytes.len() as i32
        || unsafe { output_finish(slot) } != 0
    {
        return Err(CSV);
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn publish(bytes: &[u8]) -> Result<(), Failure> {
    let path = std::env::args_os().nth(2).ok_or(CSV)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| CSV)?;
    file.write_all(bytes).map_err(|_| CSV)
}

fn main() {
    let result=execute().unwrap_or_else(|(code,message)| serde_json::to_vec(
        &json!({"contractVersion":VERSION,"ok":false,"data":null,"error":{"code":code,"message":message}})
    ).expect("bounded failure report"));
    use std::io::Write;
    if std::io::stdout().write_all(&result).is_err() {
        std::process::exit(1);
    }
}
