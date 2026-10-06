fn main() {
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some("media-benchmark") {
        zi_devtools::clipboard::media_benchmark().expect("synthetic media benchmark");
        return;
    }
    #[cfg(windows)]
    zi_devtools::clipboard::isolated_fixture().expect("isolated Windows clipboard fixture");
}
