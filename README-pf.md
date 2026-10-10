# rustdesk-pf — headless RustDesk TCP port forwarding

`rustdesk-pf` is a small binary (crate `crates/rustdesk-pf`) that speaks the
original RustDesk rendezvous / relay / login protocol, but only does **TCP port
forwarding**. It has no GUI, no X11/Wayland, no audio, no clipboard, no screen
capture, so it runs on a plain Linux server.

* `serve`   — the controlled side. It registers an ID with hbbs and accepts port-forward
  sessions from the **official RustDesk client** or from `rustdesk-pf connect`.
  It can forward to any `host:port` the server can reach (e.g. `127.0.0.1:22`
  or another LAN machine).
* `connect` — the controlling side, like `ssh -L`. It opens local listeners and tunnels each
  accepted TCP connection to the peer.
* `login`   — logs in to the API (`POST /api/login`) and stores `access_token`.
* `id`      — prints ID and effective settings.

Transport: TCP hole punch / same-LAN direct first, then the relay (hbbr) as fallback, with
the same NaCl key exchange and encryption as upstream. The serve side only makes
**outbound** connections, so it works behind campus/corporate NAT with no inbound port.

## Build

```bash
cargo build -p rustdesk-pf --release   # no gtk/glib/xdo/pulse needed
install -m755 target/release/rustdesk-pf /usr/local/bin/
```

## Configuration

Every flag has an environment variable:

| flag | env | meaning |
|---|---|---|
| `--config` | `RUSTDESK_PF_CONFIG` | state dir (ID, keypair, password hash, token) |
| `--server` | `RUSTDESK_PF_SERVER` | ID server `host[:port]` |
| `--relay` | `RUSTDESK_PF_RELAY` | relay `host[:port]` (default ID port+1) |
| `--key` | `RUSTDESK_PF_KEY` | server public key |
| `--password` (serve) | `RUSTDESK_PF_PASSWORD` | permanent device password |
| `--password` (connect) | `RUSTDESK_PF_PEER_PASSWORD` | peer's device password |
| `--api` | `RUSTDESK_PF_API` | API base URL |
| `--api-host` | `RUSTDESK_PF_API_HOST` | Host header when `--api` is an IP |
| `--user` | `RUSTDESK_PF_ACCOUNT_USER` | API account |
| `--account-password` | `RUSTDESK_PF_ACCOUNT_PASSWORD` | API password |
| `--token` | `RUSTDESK_PF_TOKEN` | pre-obtained access_token |

Logs go to stderr (`RUST_LOG=info` default; `debug` for detail), so journald collects them.

## Older-glibc hosts: static musl build / 旧 glibc 主机静态编译

```bash
sudo apt install musl-tools            # provides musl-gcc (used by .cargo/config.toml)
rustup target add x86_64-unknown-linux-musl
cargo build -p rustdesk-pf --release --target x86_64-unknown-linux-musl
file target/x86_64-unknown-linux-musl/release/rustdesk-pf   # "statically linked"
```

OpenSSL is vendored (`openssl` feature `vendored`), so the binary needs no host libssl/glibc.

### Non-root user-mode deploy / 无 sudo 用户态部署

```bash
scp target/x86_64-unknown-linux-musl/release/rustdesk-pf deploy/rustdesk-pf-user.sh user@host:
ssh user@host
mkdir -p ~/bin ~/.config/rustdesk-pf && install -m755 rustdesk-pf ~/bin/
install -m600 /dev/null ~/.config/rustdesk-pf/rustdesk-pf.env
$EDITOR ~/.config/rustdesk-pf/rustdesk-pf.env    # see deploy/rustdesk-pf.env.example
sh rustdesk-pf-user.sh restart                   # nohup; also: start|stop|status|log
```

The script exports the env file, sets `RUSTDESK_PF_CONFIG=~/.config/rustdesk-pf/state`
and logs to `~/.config/rustdesk-pf/serve.log`. With `systemd --user` available you can
instead copy `deploy/rustdesk-pf.service` to `~/.config/systemd/user/` (drop `User=`,
point `EnvironmentFile`/`ExecStart` at the paths above) and run
`systemctl --user enable --now rustdesk-pf` (`loginctl enable-linger` to survive logout).

## Controlled server (systemd, non-root)

```bash
sudo useradd --system --home /var/lib/rustdesk-pf --shell /usr/sbin/nologin rustdesk-pf
sudo install -d -m700 /etc/rustdesk-pf
sudo install -m600 deploy/rustdesk-pf.env.example /etc/rustdesk-pf/rustdesk-pf.env
sudoedit /etc/rustdesk-pf/rustdesk-pf.env        # server, key, relay, password
sudo install -m644 deploy/rustdesk-pf.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now rustdesk-pf
journalctl -u rustdesk-pf -f                     # shows "id=..." and "RegisterPk OK"
sudo -u rustdesk-pf RUSTDESK_PF_CONFIG=/var/lib/rustdesk-pf rustdesk-pf id
```

The ID is kept in the state dir and survives restarts. The daemon re-registers every
few seconds and restarts its register loop after network errors. systemd `Restart=always`
covers crashes.

## Connecting with the official RustDesk client / 官方客户端

1. Settings → Network → ID/Relay server: same ID server, relay, key (and API + account
   login if your provider requires it).
2. **Recommended — pass the password on the command line / 推荐用命令行带密码打开：**

   ```bash
   # Flutter client (1.2+)
   rustdesk.exe --port-forward <ID> --password <DEVICE_PASSWORD>
   # older Sciter client (positional form)
   rustdesk.exe --port-forward <ID> <DEVICE_PASSWORD>
   ```
3. In the tunnel window **Add** a mapping: local `2222` → `localhost:22`. Keep the window open.
4. `ssh -p 2222 <user>@127.0.0.1`.

Why not the GUI password box? / 为什么 GUI 不弹密码框：

- Choosing **TCP tunneling** from the main window does *not* prompt for a password when the
  window opens. The prompt only appears after a mapping is added *and* a local app
  connects to the local port. 选「TCP 隧道」时不会立刻要密码；要等添加映射且本地有程序连上来才弹。
- `rustdesk-pf serve` accepts **only PortForward logins**; a normal remote-desktop
  *Connect* is rejected, so you cannot "connect first to save the password".
  本服务只接受端口转发登录，普通「连接」会被拒，无法先连接一次保存密码。
- The client first tries an empty/saved password; serve answers *Wrong Password* and closes,
  so manual entry in the GUI can be flaky. Use `--password` as above.
  客户端先用空密码试，被拒后连接关闭，GUI 手动输入不稳定，建议用 `--password`。

Concurrency / 并发: the official client and `rustdesk-pf connect` can be connected to the
same serve at the same time without conflict. The client's "port-forward mux" probe gets
"no mux" and falls back to one tunnel per TCP connection (mux off also works). Several SSH
sessions through one mapping run simultaneously; on a slow relay each new connection may
take several seconds (observed 10–20 s) to open.

## Connecting with rustdesk-pf

```bash
export RUSTDESK_PF_SERVER=203.0.113.10:21116 RUSTDESK_PF_RELAY=203.0.113.10:21117 \
       RUSTDESK_PF_KEY=... RUSTDESK_PF_PEER_PASSWORD=...
# if the provider requires an account (error "please log in"):
export RUSTDESK_PF_API=https://api.example.com:21114 RUSTDESK_PF_ACCOUNT_USER=me \
       RUSTDESK_PF_ACCOUNT_PASSWORD=...
rustdesk-pf connect 123456789 -L 127.0.0.1:2222:127.0.0.1:22   # spaces in the ID are OK
ssh -p 2222 user@127.0.0.1
```

Each accepted local connection gets its own tunnel, so several `ssh`/`scp` sessions at once
are independent. `-L` can be repeated, and the target can be any host the peer can reach
(`-L 13389:192.168.1.50:3389`).

`~/.ssh/config`:

```
Host campus
    HostName 127.0.0.1
    Port 2222
    User me
    # keep rustdesk-pf connect running (tmux/systemd --user), or start it on demand:
    # ProxyCommand sh -c 'pgrep -f "rustdesk-pf connect" >/dev/null || (rustdesk-pf connect 123456789 -L 127.0.0.1:2222:127.0.0.1:22 >/tmp/pf.log 2>&1 &); sleep 3; exec nc %h %p'
```

## Campus / restricted network notes

* **Outbound only.** serve needs outbound UDP+TCP to the ID port and TCP to the relay port.
  No inbound port and no UPnP. If UDP is blocked, registration fails (stock hbbs only accepts
  RegisterPk over UDP).
* **Fake-ip DNS** (Clash/sing-box TUN, some campus resolvers) resolves hostnames to
  `198.18.x.x`, which breaks RustDesk's own TCP/UDP. Use the provider's **real IP** for
  `--server`/`--relay`, and for HTTPS APIs use the IP in `--api` plus `--api-host <hostname>`.
  (Find the real IP with DoH: `curl -s 'https://dns.google/resolve?name=HOST&type=A'`.)
* Custom/commercial servers often **move the relay** (e.g. the relay is on a different host
  than ID+1). If relay connects fail, ask the provider or read `relay=` from the
  `PunchHoleResponse` line in `RUST_LOG=info` output, and pass it as `--relay`.
* Direct punching rarely works across two NATs. The relay fallback is automatic.

## Security

* The device password is the only thing between the internet and every port the server
  can reach. Use a long random password, and keep it in the env file (chmod 600), not in argv.
* Full-LAN forwarding is allowed by design. If you don't want that, put the server on a
  network segment where that is acceptable, or firewall the `rustdesk-pf` user
  (`iptables -m owner --uid-owner rustdesk-pf`) to the hosts/ports you intend.
* Run as the dedicated unprivileged user (the unit above). Root is never needed.
* Prefer SSH keys and `PasswordAuthentication no` on the forwarded sshd. The tunnel is
  encrypted, but treat it like any other path to sshd.
* API account passwords/tokens: env file or `rustdesk-pf login` (token stored in the state
  dir). Never commit them.

## Not supported

Remote desktop, file transfer, audio, clipboard, UDP forwarding, RDP launcher, and
mux tunnels on the serve side (clients fall back automatically). UDP/KCP, IPv6 and WebRTC
punching are not implemented. TCP punch, same-LAN direct, and relay are.
