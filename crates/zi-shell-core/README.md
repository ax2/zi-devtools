# zi-shell-core

Shared Rust implementation of Zi DevTools file-action definitions, Windows argument quoting, classic-menu plans, installation lifecycle models, bounded registry inspection, and native registry transactions. The desktop imports this implementation through compatibility modules; implementations are not copied into a separate installer path.

The core has no desktop UI, tool algorithms, services, networking, database, process-launching, or Windows Installer execution dependency. The Windows MSI module reads package identity and registered component ownership; execution remains in the desktop update adapter. Manual menu writes still require an explicitly prepared ticket; importing the library does not perform registry writes.

The library version is independent of the desktop version. Manual maintenance takes the product version explicitly, and the desktop adapter supplies its own package version. The existing `ui-preview` feature supplies controlled, non-applicable tickets without introducing a UI dependency.

This extraction does not implement an MSI custom-action DLL, installer opt-in, Windows Installer rollback, multi-selection, directory receivers, IPC, or modern Explorer menus. In particular, installer impersonation and rollback ordering require separate integration and actual disposable MSI transactions before installer acceptance.
