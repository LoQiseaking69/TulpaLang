# Tulpa Link: robotics middleware

Tulpa Link connects the parts of a robot system: robots, sensors, controllers and tools on
one machine or across a network. It is built for Tulpa and it is not a copy of ROS 2 or DDS:

- **Units are part of the type.** A topic declared as `[m/s]` only accepts speeds. A subscriber
  that declares `[m]` for the same topic is reported as incompatible when the nodes discover
  each other. Values travel in SI, so a `[cm]` subscriber and a `[m]` publisher interoperate
  exactly.
- **The same program runs on a simulated network.** The default transport is an in-process
  network on the virtual clock with seeded latency, jitter, loss and partitions, so a
  distributed test is reproducible.
- **No broker, no IDL, no code generation.** Nodes discover each other; a schema is a Tulpa
  value.
- **Remote data works with the language's safety constructs.** Every topic exposes a signal, so
  `when` rules and `always` envelopes can watch data from another machine.

```tulpa
import "std/link"

let rover = link.node("rover")
let speed = rover.advertise("/rover/speed", "[m/s]")

let base = link.node("base")
let seen = base.subscribe("/rover/speed", "[km/h]", deadline: 100[ms])
always seen.signal.get() == nil or seen.signal.get() <= 5[km/h] else { base.call("/rover/stop") }

link.spin_for(300[ms])        # let the nodes discover each other
speed.publish(0.4[m/s])
link.spin_for(10[ms])
print(seen.latest)            # 1.44 km/h
```

## Nodes and transports

`link.node(name, domain: 0, transport: nil, peers: [])`

| Transport | Use |
|---|---|
| `"sim"` (default) | Every node in the program shares a deterministic network. Shape it with `link.network(latency: 2[ms], jitter: 1[ms], loss: 0.1, seed: 7)` and cut links with `link.partition(a, b)`. |
| `"udp"` | Real sockets. Nodes on one host and on the same LAN find each other by themselves. Pass `peers: ["10.0.0.7"]` for hosts that broadcast does not reach. |

Set `TULPA_LINK=udp` to switch a whole program without editing it. Nodes only see nodes in the
same `domain` (0 to 200 on UDP).

A node does its work when it spins: `node.spin()`, `link.spin_all()`, or
`link.spin_for(duration, period)`. Call one of them from your control loop.

## Topics

```tulpa
let t = node.advertise(name, schema, reliable: false, latched: false, keep: 32)
t.publish(value)

let s = node.subscribe(name, schema, handler, reliable: false, deadline: nil)
s.latest; s.count; s.from; s.stamp; s.signal      # handler(value, meta) is optional
```

**Schemas**

| Schema | Accepts |
|---|---|
| `"any"` | anything that can be sent; compatible with every schema |
| `"int"`, `"float"`, `"bool"`, `"str"` | that type |
| `"[m/s]"` | a quantity with that dimension; received values display in the declared unit |
| `["[m]"]` | a list of that schema |
| `{x: "[m]", ok: "bool"}` | a record with exactly those fields |

A value that does not fit is an error at `publish`, for example
`/rover/speed: expected a quantity in [m/s], got a quantity in [m]`.

**Quality of service**

| Option | Effect |
|---|---|
| best effort (default) | Late and duplicate messages are dropped; nothing is resent. For sensor streams. |
| `reliable: true` (both ends) | Every message arrives, in order. The publisher keeps `keep` messages for resending. |
| `latched: true` | A subscriber that joins late receives the last value. For maps and configuration. |
| `deadline: 100[ms]` | A `deadline` event fires if no message arrives for that long. |

## Services and parameters

```tulpa
arm.serve("/arm/home", |req| { home(); "ok" })
let reply = base.call("/arm/home", nil, timeout: 2[s])
```

Requests are resent until answered, and a server runs each request once. An error raised by
the handler is raised at the caller. `call` raises if nobody serves the name or the timeout
passes.

```tulpa
rover.param("speed_limit", 1.5[m/s])
base.get_param("rover", "speed_limit")
base.set_param("rover", "speed_limit", 1[m/s])      # 1[m] would be rejected: wrong dimension
```

## Events and the graph

`node.on_event(f)` receives `{kind, ...}` dicts: `peer_joined`, `peer_lost` (after 1.5 s of
silence, or at once when the peer closes), `deadline`, and `incompatible` (with the reason).
`node.peers()` lists the nodes currently seen. `node.graph()` returns every node with the
topics it publishes and subscribes to and the services it offers.

From a terminal, on a UDP network:

```
tulpa link graph [--domain N]             # nodes, topics, services
tulpa link echo /rover/speed [--domain N] # print messages as they arrive
```

HermiSim shows the same graph in its Network dock.

## Wire format

One UDP datagram per packet, little-endian:

```
"TL" | version u8 = 1 | kind u8 | domain u16 | node id u64 | body
```

| Kind | Body |
|---|---|
| 1 announce | name, then per endpoint: role (0 pub, 1 sub, 2 service), topic, schema hash u32, flags (1 reliable, 2 latched), last sequence u64 |
| 2 data | topic, sequence u64, stamp f64, value |
| 3 nack | topic, count u16, sequences |
| 4 request | service, request id u64, value |
| 5 reply | request id u64, ok u8, value |
| 6 bye | nothing |

Strings are a u16 length plus UTF-8. Values are tagged: 0 nil, 1 false, 2 true, 3 int (i64),
4 float (f64), 5 string (u32 length), 6 list (u32 count), 7 dict (u32 count, key/value pairs),
8 quantity (f64 in SI, then eight i8 dimension exponents: m, kg, s, A, K, mol, cd, rad).

Nodes announce every 250 ms. On UDP, a node binds the first free port of
`7400 + 16 × domain ... + 15` and announces to those ports on localhost, on the broadcast
address and on each `peers` address. Data is sent directly to matching subscribers.

## On a microcontroller

`import "std/link"` works in firmware. A program that uses it runs on the
[Tulpa VM for firmware](embedded.md#the-whole-language-on-a-microcontroller), which contains a
C implementation of the node: the same wire format, discovery, schemas, reliable and latched
delivery, services and parameters. It is tested against the host implementation in both
directions (a firmware node and a host node exchanging a reliable topic and a service call).

| Board | Transport | How it reaches the fleet |
|---|---|---|
| ESP32 family | `"udp"` (the default) | Wi-Fi. Set the network in `idf.py menuconfig` → Tulpa (`TULPA_WIFI_SSID`, `TULPA_WIFI_PASSWORD`); the project template joins it before the program starts |
| Raspberry Pi and other Linux boards | `"udp"` (the default) | the board's network |
| RP2040 and other boards without a network stack | `"uart"` | a serial cable to a computer running `tulpa link bridge` |
| the simulated board (`--target sim`) | `"sim"` (the default) | the in-process network, with `link.network(latency:, loss:, seed:)` |

```
# firmware (Pico): a node on UART0
let node = link.node("rover", transport: "uart", port: "uart0", baud: 115200)
let speed = node.advertise("/rover/speed", "[m/s]", reliable: true)
```

```
# the computer at the other end of the cable
tulpa link bridge /dev/ttyUSB0 --baud 115200 --domain 0
tulpa link echo /rover/speed
```

The bridge is a Linux tool. It forwards every packet from the board to the UDP domain and every
packet it receives on the domain to the board, so the board's node appears in `tulpa link
graph` like any other, and topics, services and parameters work in both directions. A serial
frame is `COBS(destination node id: u64, 0 for everyone | packet | CRC-16/CCITT)` followed by
a zero byte.

`examples/firmware_vm` is a complete program: a behaviour tree, a state machine and a node,
which runs on the simulator and on a Pico unchanged.

## Limits

- Statically compiled firmware has no node; a firmware program that imports `std/link` runs on
  the Tulpa VM (see below).
- There is no encryption or authentication. Use it on networks you trust.
- A datagram carries one message, so a message has to fit in 64 KiB.
