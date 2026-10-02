# Owned Linux clipboard adapter journeys (#353)

Source preparation for [#353](https://github.com/o-psi/helm.vessel.voyage/issues/353).
This is not a test, coverage, native Windows/macOS, or human clipboard result.

The family contains 27 private self-libtest children. Each clears inherited
environment, selects private HOME/XDG/workspace and a PATH containing only owned
`wl-paste`, `xclip` and `powershell.exe` scripts. They cannot query the desktop
clipboard. Only the coordinator's `LLVM_PROFILE_FILE` passes to the libtest
child; the production reader's environment allowlist is checked separately.

Assertions cover Unicode literal text, read-only input despite retired command
denials, sandbox and unavailable workspace refusal before helpers, Wayland to
Xclip fallback, exact MIME commands and image priority/empty-image fallback,
URI/desktop copied-file decoding and invalid URI fallback, empty/unknown MIME
results, missing/nonzero utilities, private stderr, invalid UTF-8, byte limits,
deadline and cancellation before/after helper admission. Five Linux-hosted WSL
adapter cases validate constant PowerShell command dispatch, file/image/text
preference and invalid file refusal; they do not execute Windows APIs.

Actual helper PID/start-time, process group and private workspace witnesses verify
retirement, including refusal to count an unreaped matching zombie as cleaned.
The parent bounds each child to twelve seconds. On timeout it signals only
matched owned helper groups and retains private failure evidence; it does not
claim observed cleanup or a passing result. No system configuration, user
clipboard, provider, Vessel, grant or native platform outside Linux is used.

The parent and its children use the same Helm LIB executable, so no new native
binary entry point is needed. The delivery coordinator owns the sole Cargo gate:
focus `clipboard::owned_helper_tests::linux_clipboard_routes_bounds_privacy_and_cleanup_use_owned_children`
on Helm LIB, then strict applicable checks and the final full workspace coverage
measurement after final relevant edits. Retain all current objects/profiles and
the full denominator; do not publish a focused mixed report. Compilation/runtime
and coverage outcomes remain pending.
