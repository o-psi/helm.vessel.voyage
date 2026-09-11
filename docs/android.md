# Native Android Helm

Android Helm is a Kotlin / Jetpack Compose / Material 3 client connecting **directly
to Vessel** over authenticated WSS. It does not embed a Voyage executor, provider
adapter, WebView, or terminal emulator. Desktop Helm remains unchanged. This is the
first Android implementation wave tracked in [#260](https://github.com/o-psi/voyage/issues/260);
the Ratzilla/Tauri proposal [#259](https://github.com/o-psi/voyage/issues/259) is closed
**not planned**. Unverified acceptance remains open, not implied by a build.

## Build and install

Requirements: JDK 17, Android SDK platform 35, build-tools 35.0.0, platform-tools,
and SDK licenses accepted by the builder. The checked-in Gradle 8.11.1 wrapper
verifies the distribution SHA-256. AGP is 8.9.1, Kotlin/Compose plugin 2.1.20,
Room 2.7.1, and the Compose BOM is 2025.04.01. Minimum Android is 8.0 (API 26),
target is API 35. These version declarations are in `android/` rather than the
Rust Cargo workspace.

```sh
export JAVA_HOME=/path/to/jdk17
export ANDROID_HOME=/path/to/android-sdk
cd android
./gradlew :app:assembleDebug :app:testDebugUnitTest :vessel-client:test
./gradlew :app:connectedDebugAndroidTest
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

`connectedDebugAndroidTest` requires a running device or emulator. For an emulator,
install `emulator` and `system-images;android-35;google_apis;x86_64` with
`sdkmanager`, create an AVD with `avdmanager`, and start it before that command.
There is no hosted CI or signing-key setup. Debug APKs use the local Android debug
key; production signing/distribution is not asserted. Do not commit signing keys,
`local.properties`, SDKs, build output, or grants.

## Connect and use

1. Provision a **scoped grant** using an existing authorized Vessel administration
   surface. Android does not mint grants or run the loopback pairing flow.
2. Make Vessel's full-duplex `/v1/vessel/socket` endpoint reachable over WSS, using
   a trusted certificate (optionally through your own private network). Preserve
   WebSocket upgrades and the `voyage.vessel.v1` subprotocol at the TLS proxy.
3. Independently obtain the expected Vessel UUID, grant UUID, and scoped token.
   Enter them in **Connection** with `wss://host/v1/vessel/socket`. Do not put
   credentials in the URL. No insecure HTTP/WS fallback, trust-all certificates,
   or hostname-verification bypass exists.
4. Select a visible voyage. The grant and executing voyage enforce all authority;
   a disabled/unavailable operation is not an invitation to broaden it.
5. **Send** submits a new turn when idle. **Steer** is distinct and targets the
   observed active run/incarnation. **Cancel run** requires confirmation and
   requests remote cancellation; acknowledgement is not observed cleanup.
   Supported decisions are approval/denial and multiple-choice/custom questions.
   Approval has a separate confirmation. Unknown decision kinds are not answered.
6. **Disconnect**, Back, backgrounding, rotation, or killing the app never sends
   remote cancellation. Use **Connect/Reconnect** to restore observation.

There are no provider credentials on the phone. Provider sign-in and billing remain
on the executing machine. Never enter local loopback tokens or secrets into the
message/question composer. Prompt answers enter conversation history.

## Persistence and recovery contract

- An app-private Room database atomically inserts command UUID, Vessel/session
  identity and **exact serialized request before any dispatch**. Socket request
  IDs are separate. Duplicate IDs must match immutable payload and identity.
- A lost response remains uncertain. The client sends only receipt reads during
  recovery, including after process death. It never blindly resends an uncertain
  intent or invents a new ID for a retry. Unknown receipts remain pending and
  block another action in the selected voyage. **Check receipts** performs a
  bounded recovery pass (up to 128); repeated checks do not dispatch mutations.
- `RESOLVED` means admission/refusal was observed, not that the remote run finished
  or cleanup completed. Room prevents a late failure from regressing this state.
- Cache rows are scoped by Vessel UUID. Reconnect/resume checks canonical snapshot
  and revision-pinned history on one socket/incarnation. Invalidation events are
  never appended as message text. A three-second foreground reconciliation also
  checks changes; disconnect/gap/failure marks cached observations **STALE**.
- Only reconciled `live_text` is displayed as provisional output. Canonical history
  replaces it, rather than duplicating streaming text into saved messages. Earlier
  history is paged at a pinned revision. Truncated messages can be expanded up to
  4 MiB; larger messages explicitly require another Helm client. Live previews
  retain the protocol's own truncation indicators.
- The connection record is AES-GCM encrypted under an Android Keystore key. Grant
  text is not saved in instance-state bundles, logs, journal entries or backups.
  Token entry uses password semantics; the activity disables screenshots/recents
  capture. Cloud backup and device transfer exclude app data. Keystore failure
  fails closed; it does not fall back to plaintext.
- Forgetting connection credentials leaves private journal/cache rows so uncertain
  commands remain recoverable with a replacement grant to the same Vessel. This
  does not revoke the remote grant. App uninstall deletes local recovery data.

The database is protected by Android app-private storage, **not SQLCipher**. The
Keystore may be software-backed on an emulator; hardware-backed/StrongBox storage
is not claimed. Screenshots restrictions and backup exclusions are defense in
depth, not protection against a compromised device. Kotlin immutable strings
cannot guarantee complete in-memory secret zeroization.

## Verification and remaining limits

Verified on 2026-09-11 using Linux, JDK 17, Gradle 8.11.1 and an Android 35
(API 35, x86_64 Google APIs) emulator with KVM:

| Check | Observed result |
| --- | --- |
| `:app:assembleDebug` | APK built and installed successfully |
| `:vessel-client:test` | 21 passed, 0 failed/errors/skipped (12 client, 9 transport) |
| `:app:testDebugUnitTest` | 3 passed, 0 failed/errors/skipped |
| `:app:connectedDebugAndroidTest` | 13 passed, 0 failed/errors/skipped |
| `:app:lintDebug` | Passed with 13 warnings: 10 pinned dependency updates, 2 intentional synchronous preferences commits, 1 API-qualified launcher resource |
| Separate `aSeed` / force-stop / `bRecover` instrumentation | Both passed (1 test each); journal/cache and Keystore record survived; no app PID observed after force-stop |
| Native IME/Back | ADB touch/input opened the real IME; Back changed `mInputShown` true → false while MainActivity remained resumed |
| Source/docs checks | Git whitespace check, relative documentation paths and generated Room schema checked |

Final combined command: `./gradlew :app:assembleDebug :app:testDebugUnitTest
:vessel-client:test :app:connectedDebugAndroidTest :app:lintDebug --console=plain`
(on one shell line), exit 0. Local evidence is under `android/build/evidence/`;
APK, XML results and logs are also retained in the primary checkout's ignored
`target/android-260-delivery/` before worktree retirement. Initial setup failures
(corrupt interrupted Gradle lock, missing test BOM, Kotlin inferred recursion)
were corrected, not counted as passes. The first standalone process-death command
found no installed test APK after Gradle's cleanup; installing both APKs resolved
that prerequisite before the passing split run.

JVM tests alone are not Android runtime, device security, or public-network evidence.

- Client tests cover public JSON envelopes, TLS/identity checks, durable/correlation
  ID separation, lost responses, journal failures, receipts and reconnect fencing.
- Android instrumentation exercises Room insert races, immutable identity and
  reopen, monotonic receipt state, real Keystore encryption/tamper handling,
  Compose setup/recreation and a ViewModel/client/Room lifecycle journey with a
  **synthetic transport**. That journey is not a deployed-WSS acceptance test.
- `ProcessDeathTest` can be run in two actual app processes with an intervening
  force-stop, independently of the ordinary combined instrumentation run:

  ```sh
  adb install -r app/build/outputs/apk/debug/app-debug.apk
  adb install -r app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
  adb shell am instrument -w -e class dev.helm.android.ProcessDeathTest#aSeed \
    dev.helm.android.test/androidx.test.runner.AndroidJUnitRunner
  adb shell am force-stop dev.helm.android
  adb shell am instrument -w -e class dev.helm.android.ProcessDeathTest#bRecover \
    dev.helm.android.test/androidx.test.runner.AndroidJUnitRunner
  ```

A trusted deployed WSS Vessel/grant, real handset/OEM backup security, and a complete
real-network loss/resume journey still need actual evidence. No live provider or
paid task is required by the offline tests. OkHttp 4.12 rejects oversized frames
in the application callback, **after** WebSocket fragment aggregation; it cannot
bound pre-callback memory used by a malicious TLS-trusted peer. The client bounds
requests, queues, deadlines and application-level frame/event sizes, not all
underlying parser allocation. See the [client API notes](../android/vessel-client/README.md).

Deferred: new-voyage/configuration administration, background push (separate active
[#72](https://github.com/o-psi/voyage/issues/72)), QR pairing, private PTY,
attachments, iOS, and browser clients. No capability is supplied by a demo
transport in production; production constructs the real OkHttp Vessel client.
