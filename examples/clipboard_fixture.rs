fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some("media-benchmark") {
        return match zi_devtools::clipboard::media_benchmark() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Synthetic clipboard media benchmark failed: {error}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    #[cfg(windows)]
    {
        match zi_devtools::clipboard::isolated_fixture() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Isolated Windows clipboard fixture failed: {error}");
                std::process::ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("The isolated clipboard fixture is available only on Windows.");
        std::process::ExitCode::FAILURE
    }
}
