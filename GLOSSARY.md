# OmniPort

OmniPort is a virtual serial port router: it moves serial traffic between OS serial
endpoints, routes and mutates it in software, and records it for inspection.

## Language

### Traffic

**Frame**:
One complete unit of serial traffic after the framing rules are applied: a payload,
a timestamp, the sampled pin state and status flags.
_Avoid_: packet, event, message, sample

**Payload**:
The bytes carried by a frame, excluding framing delimiters and checksum.
_Avoid_: data, body, contents

**Pin State**:
The sampled value of the modem control signals (RTS, CTS, DTR, DSR, DCD, RI and BREAK)
at the moment a frame is captured.
_Avoid_: control lines, handshake, modem bits

**Frame Flags**:
The per-frame status bits: direction (TX/RX), parity/framing/overrun/CRC errors,
fault-injected and pin-change markers.
_Avoid_: status, metadata, errors

### Capture and inspection

**Session**:
One capture run owned by one daemon instance: the frames recorded from start to stop,
addressed by a monotonically increasing row index.
_Avoid_: capture (the noun), log, run, recording

**Capture**:
The act and artefact of recording frames into a session; "the capture" is the session's
recorded frames, not the GUI.
_Avoid_: trace, dump

**Row**:
A frame's position in a session, addressed by its row index; the unit a viewport requests.
_Avoid_: packet index, record, line

**Viewport**:
The contiguous window of rows a client has asked to display, identified by a start row
and a row count.
_Avoid_: page, chunk, window (a window is the on-screen area)

**Viewport Batch**:
The wire form of one viewport: a total row count followed by the requested rows in row order.
_Avoid_: page, payload, response

**Demo Stream**:
Synthetic frames produced to exercise the workbench without a live daemon; always labelled as
demo data and never presented as a capture session.
_Avoid_: mock data, fake traffic, sample stream

### Routing

**Endpoint**:
A serial-side terminus that frames can be sent to or received from: a physical UART, a
virtual serial port, a network peer, a script mock or the Wireshark capture stream.
_Avoid_: port, node, device, peer

**Port Path**:
The operating-system name for an endpoint's serial device (for example `COM3` or
`/dev/ttyUSB0`).
_Avoid_: port, device path, address

**Route**:
A directed connection from one endpoint to another, optionally carrying a fault injector.
_Avoid_: connection, edge, link, channel

**Topology**:
The set of endpoints and routes that make up a session's routing graph.
_Avoid_: graph, network, map

**Splitter**:
A route topology that fans one source endpoint out to several destinations (1:N).
_Avoid_: broadcaster, tee

**Merger**:
A route topology that funnels several source endpoints into one destination (N:1),
preserving frame atomicity.
_Avoid_: aggregator, join

**Fault Injector**:
A per-route modifier that delays, corrupts, drops or disconnects frames to emulate a
faulty link.
_Avoid_: simulator, chaos, error injector

**Dissector**:
The zero-compile decoder that maps frame payload bytes onto a packed C struct definition.
_Avoid_: parser, decoder, unpacker

**Scenario**:
A declarative YAML description of a CI test run: endpoints, routes, faults and assertions.
_Avoid_: test, case, job, plan

**Daemon**:
The headless `omniportd` process that owns sessions, endpoints and routes independently
of any client.
_Avoid_: server, backend, service
