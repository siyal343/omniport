# ADR-0002: Hybrid Windows driver strategy

- **Status:** Accepted

## Decision

Wrap the signed `com0com` driver via native `SetupAPI`/`DeviceIoControl` in Phase 1
while building a SignPath-signed Rust UMDF2 (`SerCx2`) driver behind the same HAL
trait.
