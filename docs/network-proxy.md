# Network proxy

DegenView follows a proxy for everything it sends over the internet: market data, live price feeds,
updates, link previews, webhooks, AI requests and chart sharing. It was added for users whose VPN or
network publishes a proxy (a Windows user running Karing in system-proxy mode, say) and who could
only start the app from a launcher that set `HTTPS_PROXY`. Code: `src/service/net/proxy/`; page:
Settings ▸ Network (`src/ui/settings/network/`).

## The setting

`ProxyConfig`, the local `net.proxy` setting (never synced: it names this Mac's own network;
classified in `sync::record::policy`).

| Mode | Behaviour |
| --- | --- |
| System (default) | The `ALL_PROXY` / `HTTPS_PROXY` / `HTTP_PROXY` variables (and lowercase), then the operating system's proxy. |
| Manual | The server typed on the page: HTTP (`CONNECT`) or SOCKS5, host, port, optional user name. |
| Off | Direct, whatever the system or environment says. |

The manual proxy's password is never in SQLite: `keychain::proxy_password::ProxyPasswordStore` (service
`es.nico.degenview.network`). It is read only when a user name is set, so nobody without one is
prompted. `DEGENVIEW_NO_KEYCHAIN` keeps it in memory.

The no-proxy list (`no_proxy`, comma/semicolon/space separated) adds to every mode but Off. A bare
domain also covers its subdomains (as `curl` reads `NO_PROXY`); `*.x`, `.x`, `10.1.*` and `*` work;
address ranges (`10.0.0.0/8`) are refused with a message, because the matcher is `ureq`'s.
Local hosts **always** connect directly, decided by what the host *is* (`bypass::is_local`):
loopback, `10/8`, `172.16/12`, `192.168/16`, `169.254/16`, IPv6 `::1`, `fe80::/10`, `fc00::/7`
(also as IPv4-mapped), `localhost`, `*.localhost` and `*.local`. A public name that merely starts
like an address, `10.example.com`, goes through the proxy. `ureq` can only match the spelling of a
host, so it cannot make this decision; the agents are picked per request instead (below).

What the page checks depends on what it shows: the port only for a manual proxy, the no-proxy list
unless the proxy is Off. A leftover typo in a hidden field never blocks switching mode.

`Spec.host` is the bare name or address: an IPv6 proxy has no brackets in it (`::1`, however it was
typed or read from a URL), so a socket can dial it as it is; `Spec::build` brackets it for the URL
`ureq` wants, and `describe` for the status line. Credentials need no escaping by us: `ureq`
percent-encodes them when it builds the proxy. A complete manual proxy that still cannot be built is
shown as a problem on the page (`Status::problem`); it is never dropped silently into direct
connections.

## Where the OS proxy comes from

- **Windows**: `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings` (`ProxyEnable`,
  `ProxyServer`, `ProxyOverride`) through `windows-sys` (`Win32_System_Registry`, no new crate).
  `ProxyServer` is either `host:port` or `http=…;https=…;socks=…` (HTTPS entry preferred, then HTTP,
  then SOCKS5). `<local>` in `ProxyOverride` is dropped (local hosts bypass anyway). A PAC script
  (`AutoConfigURL`) is **not** followed.
- **macOS**: `scutil --proxy` (HTTPS, else HTTP, else SOCKS; `ExceptionsList`). A process spawn: it
  runs when the proxy is applied or refreshed, on a worker, never in a frame.
- **Linux**: no system setting; environment and manual only.

Parsers are pure and tested on every platform (`os/scutil.rs`, `os/server_list.rs`); only the
registry call (`os/registry.rs`) is `cfg(windows)`.

## How it reaches the code

- `net::proxy::current` holds the process-wide state: the config, the resolved `Resolved`
  (server, `ureq::Proxy` that knows what to skip, source) and a generation that counts changes.
  Nothing is resolved before `proxy::init(&store)` / `proxy::apply`, so tests and tools connect
  directly. In System mode it is read again when older than 30 s: `proxy::refresh_if_stale`, which
  blocks, is called only where a worker is about to connect (`FollowingAgent::get_for`,
  `config_builder_for`, `tunnel::dial`, a live feed's route check), so a VPN client switching its
  proxy on or off is followed without a restart. `snapshot`, `route`, `generation` and `status` are
  cheap reads.
- `init` runs at both entry points: `run_gui_with` and `run_alert_agent`. The alert agent is a
  separate process: its loop calls `proxy::follow(&store)`, which every 2 s reads the saved setting
  and applies it when it changed, and re-reads the password every 30 s for a manual proxy with a
  user name (`ProxyPasswordStore::read_fresh`, past the per-process cache), so a change made in
  the app reaches it without a restart.
- **Every HTTP agent is built for the URL it will call.** Per call: `net::config_builder_for(url)`
  (a `ureq` builder with the proxy applied unless the host skips it), never
  `ureq::Agent::config_builder()` (a test fails if non-test code does). Kept in a struct or a
  static: `net::agent::FollowingAgent::get_for(url)`, which holds a proxied and a direct agent per
  proxy generation (rebuilt after a change) and hands out the one the host calls for: `net::http`,
  the webhook, snapshot and FX transports. Agents built per call (`update::download`,
  `annotation::link_fetch`, `llm::HttpProvider`) use `config_builder_for`. A redirect keeps the
  first hop's route (the sites that carry credentials already refuse redirects).
- **WebSockets** (`net::ws::connect`) open their TCP stream with `proxy::dial`, which tunnels
  through the proxy unless it skips the host, and then speak TLS over the tunnel
  (`tunnel/mod.rs`): HTTP `CONNECT` with Basic auth, SOCKS5 no-auth and user/password (the host
  name goes to the proxy, not an address, unless it is one). SOCKS4 and `https://` proxies work for
  REST (`ureq`) but not for live feeds: the error says so. Tests run against in-process fake
  proxies. A feed reconnects when the setting changes: `ws::connect` returns the proxy generation
  it dialled under (`Route`), `Health::new` keeps it, and `Health::read` checks it once a second
  and fails with "the proxy setting changed", which the feed loops already answer by reconnecting
  after their backoff.
- The Settings host saves the setting on the frame (cheap), then applies it on a worker
  (`SNAPSHOTS` queue; reading the system proxy and the Keychain block), one at a time so saves end
  in order.

## Not covered

- **`degenview-brokers`** (live trading) is an external pinned crate with its own HTTP client; it
  does not follow this setting yet. The page says so.
- CloudKit (iCloud Sync) uses the system's networking and its own proxy handling.
- PAC scripts and WPAD; per-host proxy rules beyond the no-proxy list.

## Verified and not

Unit-tested: resolution order, env and OS parsers, bypass rules, the tunnels against fake proxies,
the setting round trip, the page's rules, and that no agent bypasses the proxy setting (`config_builder_for` / `get_for`). Looked at in a
picture: the Network page in System, Manual and Off, both themes. **Not exercised**: a real Windows
machine (the registry reader is only type-checked there), Karing, a real HTTP/SOCKS proxy end to end
against a market provider, and a live WebSocket feed through a proxy.
