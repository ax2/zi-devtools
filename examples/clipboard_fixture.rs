fn main() {
    #[cfg(windows)]
    zi_devtools::clipboard::isolated_fixture().expect("isolated Windows clipboard fixture");
}
