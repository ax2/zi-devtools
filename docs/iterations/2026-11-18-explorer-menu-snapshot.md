# Explorer popup snapshot diagnostics (Studio #80)

## Scope and original evidence

- Tracking: https://github.com/ax2/zicode-studio-desktop/issues/80, planned Studio milestone v0.6.6. This changes only the independent provider's test probe, not Studio runtime or the standalone product version.
- Original PR failure remains FAILURE: https://github.com/ax2/zi-devtools/actions/runs/44228311014/job/134546850872 (`GetMenuItemCount=-1` while reading the background popup).
- No product/menu changes have been identified as causing that failure. In particular this is not evidence that the JSON WASI plugin caused it, nor proof of one particular Explorer timing race.

## Small reproducer and fix

The original nested `labels` function was extracted with Python AST and called on a real process-owned HMENU after `DestroyMenu`. It reproduces exactly `Invalid visible menu item count: -1`, without creating a window or sending desktop input. Microsoft documents -1 as failure, not an empty menu: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getmenuitemcount .

The probe now reacquires the HWND and HMENU together until a valid, nonempty snapshot is available within the existing timeout. It filters popup windows to the disposable Explorer process, checks handle/item-count validity after reading, and detects a replaced or hidden popup. Retry diagnostics include the actual Win32 error and are preserved in `explorer-visible-proof.json` even on failure. Valid menus missing the product label, oversized menus and truncated labels still fail; product assertions are not relaxed. The hosted-runner guard remains required for visible Explorer interaction.

## Verification

- Windows native Python regression: **12 passed**, using only invisible, process-owned HMENUs. Includes Unicode/separators; real destroyed-handle -1 / error 1401; bounded reacquisition; permanently invalid/no menu failure; initialization; handle replacement; destruction during label read; missing product still failing; oversized/truncated menu hard failure; foreign process exclusion; developer desktop guard.
- No Cargo compilation, local MSI registration, Explorer launch/input, screenshot capture or user-process termination was performed for these regressions.
- Reproduce the focused suite with `python -B -m unittest discover -s scripts -p test_explorer_menu_snapshot.py -v` on Windows. The existing default workflow is unchanged and has no snapshot-only dispatch. No GitHub run is claimed for this patch; its local native results are the new evidence.

## Reused actual hosted background evidence and limits

The later, already completed main run https://github.com/ax2/zi-devtools/actions/runs/44231960418 (source `ef21de5cb2fae69c1492eaeecc404ccb9495009b`) has seven successful jobs. Its explorer-visible job https://github.com/ax2/zi-devtools/actions/runs/44231960418/job/134558902564 includes real background root/cascade screenshots. Both have been inspected: one Zi DevTools entry and the “分析目录空间…” action are visible, and the synthetic fixture remains unchanged. The proof records the registered `%V` command/path match and receiver exit.

Do not overstate that receipt: `runnerOpenGLUnavailable=true`, `visibleWindow=false`, and `nestedMenuActivationVerified=false` mean receiver UI launch and actual nested action activation are not proven. Also the prior proof's selected-directory screenshot/labels show a background-style menu (View/Sort by), so its boolean alone is not independent selected-directory evidence. These are historical coverage boundaries, not new Studio JSON blockers. This change has not rerun the complete MSI or product UI suite and does not erase the original failed run.
