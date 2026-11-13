# rustls-platform-verifier (local replacement)

`reqwest` 0.13 verifies TLS certificates with `rustls-platform-verifier`. On Android that crate calls Java through
JNI and needs a Kotlin component that a native app packaged with `cargo-apk` cannot include: cloning with git
panicked in `android.rs`.

This crate exposes the same interface that `reqwest` uses (`Verifier::new` and `Verifier::new_with_extra_roots`), but
verifies against Mozilla's root certificates bundled in `webpki-roots`. It is enabled with `[patch.crates-io]` in the
root `Cargo.toml`.

Trade-offs:
* It does not use the system certificate store, so a corporate CA installed on the device will not be trusted.
* The roots are only updated when `webpki-roots` is updated.
