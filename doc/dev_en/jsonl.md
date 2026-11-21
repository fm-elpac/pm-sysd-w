# IPC Protocol (jsonl module)

The `jsonl` module handles the JSONL communication protocol between init and ctl
/ log. It only encodes / decodes messages (protocol handling); it does no IO
(message sending / receiving and limits are handled by the `pa` module).

## Transport

- Linux: a UNIX socket in the current directory (for testing only).
- Windows: a named pipe, requiring permission control, local connections only.

## Protocol

- Data transport is JSONL (one JSON per line), using `serde_json`.
- After connecting, a version handshake (transport protocol version + software
  version) happens first; subsequent messages carry no version.
- Subsequent requests are two-level: the outer level names the handling
  component and whether it is read-only, the inner level names the operation.

The handshake message format and each request / response type and field are in
the `jsonl` module's code comments.

The transport protocol is an internal API, not a stable external API; BREAKING
changes may appear even in minor updates. In principle init and ctl / log must
share the same version.
