fn main() {
    println!("# IANA {}", chrono_tz::IANA_TZDB_VERSION);
    let mut names: Vec<_> = chrono_tz::TZ_VARIANTS
        .iter()
        .map(|zone| zone.name())
        .collect();
    names.sort_unstable();
    for name in names {
        println!("{name}");
    }
}
