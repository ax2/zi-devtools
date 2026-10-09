use regex_syntax::hir::{Class, HirKind};
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let hir = regex_syntax::Parser::new().parse(r"\d").unwrap();
    let HirKind::Class(Class::Unicode(class)) = hir.kind() else {
        panic!("Unicode decimal class");
    };
    let ranges = class
        .iter()
        .map(|r| format!("({:?},{:?}),", r.start(), r.end()))
        .collect::<String>();
    let path =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("unicode_decimal.rs");
    std::fs::write(path,format!("// Generated from locked regex-syntax Unicode Nd; no runtime parser.\nconst DECIMAL_RANGES:&[(char,char)]=&[{ranges}];\n")).unwrap();
}
