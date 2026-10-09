use regex_syntax::hir::{Class, HirKind};
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let mut source =
        String::from("// Locked regex-syntax Unicode classes, generated at build time.\n");
    for (name, pattern) in [("WORD", r"\w"), ("DECIMAL", r"\d")] {
        let hir = regex_syntax::Parser::new().parse(pattern).unwrap();
        let HirKind::Class(Class::Unicode(class)) = hir.kind() else {
            panic!("Expected Unicode class");
        };
        let ranges = class
            .iter()
            .map(|r| format!("({:?},{:?}),", r.start(), r.end()))
            .collect::<String>();
        source.push_str(&format!("const {name}:&[(char,char)]=&[{ranges}];\n"));
    }
    let path =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("trace_classes.rs");
    std::fs::write(path, source).unwrap();
}
