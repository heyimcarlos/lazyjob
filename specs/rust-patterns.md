# Rust Patterns We Use

Patterns adopted from zero2prod and adapted for an agent CLI.
This is a living document -- we add patterns as we adopt them.

---

## 1. lib.rs + Thin main.rs

All logic lives in the library crate (`src/lib.rs` and its modules).
`main.rs` only does orchestration: read config, build the agent, run.

**Why**: Makes everything testable. Integration tests can import from the
library directly. If logic lives in main.rs, tests can't reach it.

```rust
// src/main.rs -- this is ALL that goes here
use platano::{agent, configuration};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // setup, build, run
}
```

```rust
// src/lib.rs -- module registry, nothing else
pub mod agent;
pub mod client;
pub mod configuration;
pub mod tools;
pub mod types;
```

---

## 2. Newtype Wrappers (Parse, Don't Validate)

Wrap primitive types in structs with private fields.
The only way to construct them is through a method that validates.
Once you hold the type, it's guaranteed valid.

```rust
pub struct ApiKey(secrecy::Secret<String>);

impl ApiKey {
    pub fn parse(s: String) -> Result<Self, anyhow::Error> {
        if s.is_empty() {
            anyhow::bail!("API key cannot be empty");
        }
        Ok(Self(Secret::new(s)))
    }
}
```

**Why**: Bugs from invalid data are caught at construction, not deep in
business logic. The compiler enforces it -- you can't accidentally pass
a raw string where an ApiKey is expected.

---

## 3. Secrets with secrecy

API keys and tokens are wrapped in `secrecy::Secret<String>`.
This type implements `Debug` and `Display` as `[REDACTED]`,
so keys never leak into logs or error messages.

Access the value explicitly with `.expose_secret()`.

```rust
let key = Secret::new("sk-ant-...".to_string());
println!("{:?}", key);           // prints: Secret([REDACTED])
println!("{}", key.expose_secret()); // prints: sk-ant-...
```

**Why**: Defense in depth. One bad `dbg!()` or `tracing::info!()` call
shouldn't leak credentials.

---

## 4. Tracing Over println

Use `tracing` for all diagnostic output. Never `println!` for debugging.

- `tracing::info!` -- things the user cares about
- `tracing::debug!` -- things a developer cares about
- `tracing::error!` -- something went wrong

Use `#[tracing::instrument]` on functions to automatically create spans
with function name and arguments.

```rust
#[tracing::instrument(skip(client))]
async fn send_message(client: &Client, msg: &str) -> Result<Response> {
    tracing::debug!("sending message to API");
    // ...
}
```

**Why**: Structured, leveled logging that you can filter at runtime
with RUST_LOG=platano=debug. println disappears into the void.

---

## 5. Error Handling Split: thiserror + anyhow

- **thiserror** at boundaries -- public error enums that callers match on
- **anyhow** internally -- for chaining context on fallible operations

```rust
// Public error type at the boundary
#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error("API request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("API returned error: {status} - {message}")]
    ApiError { status: u16, message: String },

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

// Internal code uses anyhow for context
let response = client
    .post(url)
    .send()
    .await
    .context("failed to send request to Claude API")?;
```

**Why**: thiserror gives callers something to match on (is it a network
error? an API error?). anyhow gives you `.context()` for rich error
chains without boilerplate.

---

## 6. Feature-Gated Dependencies

Only pull in the features you actually use. Keeps compile times down
and avoids dragging in system dependencies like OpenSSL.

```toml
# Use rustls instead of OpenSSL
reqwest = { default-features = false, features = ["rustls-tls", "json"] }

# Only the tokio features we need
tokio = { features = ["macros", "rt-multi-thread"] }
```

**Why**: `reqwest` defaults to OpenSSL, which requires system headers
and is a common source of build failures. rustls is pure Rust.
Full tokio includes timers, fs, net, signals -- we don't need all that yet.

---

## 7. Module Re-exports (Facade Pattern)

When a module has submodules, the parent `mod.rs` keeps them private
and re-exports only the public surface.

```rust
// src/types/mod.rs
mod request;
mod response;
mod content;

pub use request::*;
pub use response::*;
pub use content::ContentBlock;
```

**Why**: Callers write `use platano::types::Message` instead of
`use platano::types::request::Message`. Internal structure can change
without breaking the public API.

---

## Patterns We'll Add Later

- [ ] Layered configuration (config crate with TOML + env overrides)
- [ ] Integration tests with mock HTTP server
- [ ] Builder pattern for complex structs
- [ ] Property-based testing

