# sorx-event-bridge

NATS event bridge for the Greentic sorx runtime.

Consumes `greentic.sorla.request.v1` messages from NATS, dispatches them to a
local sorx runtime through the `SorxInvoker` seam, and publishes
`greentic.sorla.response.v1` echoing the correlation id.

This is the sorx-side counterpart of the async runtime dispatch (`sorla.call`)
flow node in the Greentic runner. The wire contract mirrors
`greentic-types::runtime_dispatch`.

## Usage

```rust
use std::sync::Arc;
use sorx_event_bridge::{run_bridge, SorxInvoker};

let client = async_nats::connect("nats://127.0.0.1:4222").await?;
let invoker: Arc<dyn SorxInvoker> = /* your SorxInvoker impl */;
run_bridge(client, invoker).await?;
```

See `E2E.md` for the end-to-end runbook.
