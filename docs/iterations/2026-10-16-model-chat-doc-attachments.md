# Model chat PDF and DOCX attachments

## Goal

Expand the model chat workbench beyond UTF-8 text files so a user can review and explicitly send bounded text extracted from common documents.

## Change

- Reused the local document-ingestion parsers for PDF and DOCX; no new runtime or crate dependency was added.
- PDF text retains page labels; DOCX text retains paragraph labels. Legacy `.doc`, image attachments and OCR are not supported.
- Limits: four attachments, 4 MiB source file for PDF/DOCX, 64 KiB extracted text per document, 128 KiB combined attachment text. Oversized extracts are rejected as a whole.
- The UI previews the exact text snapshot before send, omits full paths, and explains that the current model service may be remote. Reading a file does not send it; sending occurs only when the user runs the model request.
- Bumped `model-chat` to 0.2.0 and synchronized the public tool catalog.

## Validation

- `cargo test --lib`: 904 passed, 27 ignored.
- `cargo test --lib chat_attachments::tests`: 3 passed.
- `cargo test --lib document_ingestion::tests`: 5 passed.
- `cargo test --lib --features ui-preview chat_attachments::tests`: 2 passed.
- `cargo clippy --all-targets --features ui-preview -- -D warnings`, `cargo fmt --check`, `python scripts/sync_tools.py --check`, and `git diff --check`: passed.
- Rendered and inspected the 1600×1125 light-theme attachment view with synthetic text/PDF/DOCX snapshots. The view shows page/paragraph provenance, byte budgets and the remote-service notice; no user file was read and no model request was made.
- Screenshot: `C:\Users\ax2\Documents\技术博客\zi-devtools\screenshots\2026-10-16-model-chat-doc-attachments\attachments-light.png` (SHA-256 `2E08F5F4AC77FF3E996978FC43B0991AFDE75755C9C1E38D97CBC011BBB32401`).
- The shared capture output folders under `target/` currently total 31,453,305 bytes; they contain the prior model-probe screenshots and subsequent UI-suite captures. Their cleanup remains blocked by the automatic safety policy noted in the previous maintenance record.

## Limits

At this iteration scanned pages still required OCR in a future increment; bounded local scanned-PDF OCR was added in `2026-10-17-scanned-pdf-ocr.md`. Image attachments and richer provider-specific formats remain open work. Real-provider compatibility was not tested.
