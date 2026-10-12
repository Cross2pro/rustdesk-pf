# rustdesk-pf — 无界面的 RustDesk TCP 端口转发

`rustdesk-pf` 是从 [RustDesk](https://github.com/rustdesk/rustdesk) 中抽出来的一个命令行程序（crate `crates/rustdesk-pf`），只做 **TCP 端口转发**。它跟 RustDesk 用同一套 rendezvous / relay / 登录协议和加密（NaCl 密钥交换），没有 GUI、X11/Wayland、音频、剪贴板或屏幕采集，所以能直接跑在普通 Linux 服务器上。

- **连接方式**：先试 TCP 打洞 / 同局域网直连，不通就自动走中继（hbbr）。
- **只出不进**：被控端只发起出站连接，放在校园网、公司 NAT 后面也不需要开放入站端口。
- **兼容官方客户端**：官方 RustDesk 客户端的「TCP 隧道 / 端口转发」可以直接连 `rustdesk-pf serve`。

子命令：

| 命令 | 作用 |
|---|---|
| `serve` | 被控端：在 ID 服务器（hbbs）注册 ID，接受官方客户端或 `rustdesk-pf connect` 发起的端口转发，可转发到本机能访问的任意 `host:port`（如 `127.0.0.1:22` 或局域网其它机器） |
| `connect` | 控制端：类似 `ssh -L`，在本地监听端口，每个连进来的 TCP 连接各开一条隧道到对端 |
| `login` | 登录 API（`POST /api/login`），保存 `access_token` |
| `id` | 打印本机 ID 和生效的配置 |

## 快速开始

```bash
# 编译（不需要 gtk/glib/xdo/pulse）
cargo build -p rustdesk-pf --release
install -m755 target/release/rustdesk-pf /usr/local/bin/

# 被控端（服务器）
export RUSTDESK_PF_SERVER=203.0.113.10:21116 RUSTDESK_PF_KEY=<服务器公钥> \
       RUSTDESK_PF_PASSWORD=<长随机设备密码>
rustdesk-pf serve            # 日志里出现 id=... 和 RegisterPk OK

# 控制端（你的电脑）
export RUSTDESK_PF_SERVER=203.0.113.10:21116 RUSTDESK_PF_KEY=<服务器公钥> \
       RUSTDESK_PF_PEER_PASSWORD=<设备密码>
rustdesk-pf connect <ID> -L 127.0.0.1:2222:127.0.0.1:22
ssh -p 2222 <user>@127.0.0.1
```

> 想让目标机器上的 AI agent（智能体）自动装好被控端？把 [docs/agent-setup-prompt.md](docs/agent-setup-prompt.md) 里的提示词复制给它即可。

## 给agent看的prompt

你是在目标机器上工作的 AI agent（智能体）：请先完整阅读 [docs/agent-setup-prompt.md](docs/agent-setup-prompt.md)，按里面的步骤把 rustdesk-pf 被控端从零装好、登录、常驻并自检，整个流程争取全自动完成，只在确实必要时（例如账户密码、设备密码、sudo 授权）向用户索要必要信息，其余一律自己判断处理；本套服务的连接信息已给出，直接使用即可：ID 服务器 `RUSTDESK_PF_SERVER=47.96.135.20:60016`，中继 `RUSTDESK_PF_RELAY=106.55.21.51:60027`，公钥 `RUSTDESK_PF_KEY=k3lsu+CTLs4OhFpq5Lh38Uvo2m8Cyb1jLz6gTCAnyCw=`，API `RUSTDESK_PF_API=https://106.55.21.51:41112`（`RUSTDESK_PF_API_HOST=api.nemocc.top`），账户 `RUSTDESK_PF_ACCOUNT_USER=cross2pro`（账户密码向用户要，绝不写进仓库、日志或命令行）。

## 配置 / 环境变量

每个参数都有对应的环境变量，部署时建议全部写进权限 600 的 env 文件，不要放在命令行里。

| 参数 | 环境变量 | 含义 |
|---|---|---|
| `--config` | `RUSTDESK_PF_CONFIG` | 状态目录（ID、密钥对、密码哈希、token） |
| `--server` | `RUSTDESK_PF_SERVER` | ID 服务器 `host[:port]` |
| `--relay` | `RUSTDESK_PF_RELAY` | 中继 `host[:port]`（默认 ID 端口 +1） |
| `--key` | `RUSTDESK_PF_KEY` | 服务器公钥 |
| `--password`（serve） | `RUSTDESK_PF_PASSWORD` | 本机固定设备密码 |
| `--password`（connect） | `RUSTDESK_PF_PEER_PASSWORD` | 对端设备密码 |
| `--api` | `RUSTDESK_PF_API` | API 地址 |
| `--api-host` | `RUSTDESK_PF_API_HOST` | `--api` 用 IP 时的 Host 头 |
| `--user` | `RUSTDESK_PF_ACCOUNT_USER` | API 账号 |
| `--account-password` | `RUSTDESK_PF_ACCOUNT_PASSWORD` | API 密码 |
| `--token` | `RUSTDESK_PF_TOKEN` | 已有的 access_token |

日志输出到 stderr（默认 `RUST_LOG=info`，排查用 `debug`），systemd 下由 journald 收集。示例配置见 `deploy/rustdesk-pf.env.example`。

## connect 用法

```bash
# 服务商要求登录账号时（报错 "please log in"）：
export RUSTDESK_PF_API=https://api.example.com:21114 RUSTDESK_PF_ACCOUNT_USER=me \
       RUSTDESK_PF_ACCOUNT_PASSWORD=...
rustdesk-pf connect 123456789 -L 127.0.0.1:2222:127.0.0.1:22   # ID 中间带空格也可以
```

- `-L` 可以写多次；目标可以是对端能访问的任意主机，例如 `-L 13389:192.168.1.50:3389`。
- 每个本地连接一条独立隧道，同时开多个 `ssh` / `scp` 互不影响。

`~/.ssh/config` 示例：

```
Host campus
    HostName 127.0.0.1
    Port 2222
    User me
    # 让 rustdesk-pf connect 常驻（tmux / systemd --user），或按需启动：
    # ProxyCommand sh -c 'pgrep -f "rustdesk-pf connect" >/dev/null || (rustdesk-pf connect 123456789 -L 127.0.0.1:2222:127.0.0.1:22 >/tmp/pf.log 2>&1 &); sleep 3; exec nc %h %p'
```

## 用官方 RustDesk 客户端连接

1. 设置 → 网络 → ID/中继服务器：填和 serve 端相同的 ID 服务器、中继、Key（服务商需要的话再填 API 并登录账号）。
2. **推荐用命令行带密码打开隧道：**

   ```bash
   # Flutter 客户端（1.2+）
   rustdesk.exe --port-forward <ID> --password <设备密码>
   # 旧版 Sciter 客户端（位置参数）
   rustdesk.exe --port-forward <ID> <设备密码>
   ```
3. 在隧道窗口里**添加**映射：本地 `2222` → `localhost:22`，窗口保持打开。
4. `ssh -p 2222 <user>@127.0.0.1`。

为什么主界面选「TCP 隧道」不弹密码框：

- 选「TCP 隧道」打开窗口时**不会**马上要密码；要等添加了映射、并且有本地程序连上本地端口，才会去连对端。
- `rustdesk-pf serve` **只接受端口转发（PortForward）登录**，普通远程桌面「连接」会被拒绝，所以没法先普通连接一次把密码存下来。
- 客户端会先用空密码 / 已存密码去试，serve 回 *Wrong Password* 后连接关闭，在 GUI 里手动输入密码不稳定。所以请用上面的 `--password`。

## 并发

- 官方客户端和 `rustdesk-pf connect` 可以同时连同一个 serve，互不冲突。
- 官方客户端的「端口转发复用（mux）」探测会得到 no mux，自动退回每个 TCP 连接一条隧道（关掉 mux 也可以）。
- 一个映射上可以同时跑多个 SSH 会话；走慢中继时每条新连接可能要等几秒到十几秒（实测 10–20 秒）才建立，建立后可以同时使用。

## 部署

### systemd（专用非 root 用户）

```bash
sudo useradd --system --home /var/lib/rustdesk-pf --shell /usr/sbin/nologin rustdesk-pf
sudo install -d -m700 /etc/rustdesk-pf
sudo install -m600 deploy/rustdesk-pf.env.example /etc/rustdesk-pf/rustdesk-pf.env
sudoedit /etc/rustdesk-pf/rustdesk-pf.env        # server、key、relay、password
sudo install -m644 deploy/rustdesk-pf.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now rustdesk-pf
journalctl -u rustdesk-pf -f                     # 看到 "id=..." 和 "RegisterPk OK"
sudo -u rustdesk-pf RUSTDESK_PF_CONFIG=/var/lib/rustdesk-pf rustdesk-pf id
```

ID 保存在状态目录里，重启不变。程序每隔几秒重新注册，网络出错后会自己重启注册循环；崩溃由 systemd `Restart=always` 兜底。

### 旧 glibc 主机：musl 静态编译

```bash
sudo apt install musl-tools            # 提供 musl-gcc（.cargo/config.toml 会用到）
rustup target add x86_64-unknown-linux-musl
cargo build -p rustdesk-pf --release --target x86_64-unknown-linux-musl
file target/x86_64-unknown-linux-musl/release/rustdesk-pf   # 显示 "statically linked"
```

OpenSSL 以 vendored 方式编进程序，目标机不需要 libssl，也不依赖系统 glibc 版本。`.cargo/config.toml` 里的 musl 设置只在指定 musl target 时生效，普通编译不受影响。

### 无 sudo 用户态部署

```bash
scp target/x86_64-unknown-linux-musl/release/rustdesk-pf deploy/rustdesk-pf-user.sh user@host:
ssh user@host
mkdir -p ~/bin ~/.config/rustdesk-pf && install -m755 rustdesk-pf ~/bin/
install -m600 /dev/null ~/.config/rustdesk-pf/rustdesk-pf.env
$EDITOR ~/.config/rustdesk-pf/rustdesk-pf.env    # 参考 deploy/rustdesk-pf.env.example
sh rustdesk-pf-user.sh restart                   # nohup 方式；还支持 start|stop|status|log
```

脚本会加载 env 文件，设置 `RUSTDESK_PF_CONFIG=~/.config/rustdesk-pf/state`，日志写到 `~/.config/rustdesk-pf/serve.log`。如果有 `systemd --user`，也可以把 `deploy/rustdesk-pf.service` 拷到 `~/.config/systemd/user/`（删掉 `User=`，把 `EnvironmentFile` / `ExecStart` 改成上面的路径），然后 `systemctl --user enable --now rustdesk-pf`；要退出登录后继续运行，执行 `loginctl enable-linger`。

## 校园网 / 受限网络

- **只需出站**：serve 需要能出站访问 ID 端口（UDP+TCP）和中继端口（TCP），不需要入站端口和 UPnP。UDP 被封的话注册会失败（标准 hbbs 只通过 UDP 接收 RegisterPk）。
- **Fake-ip DNS**（Clash/sing-box TUN、部分校园 DNS）会把域名解析到 `198.18.x.x`，导致 RustDesk 自身的 TCP/UDP 失败。`--server` / `--relay` 请填服务商的**真实 IP**；HTTPS API 用 `--api <IP>` 加 `--api-host <域名>`。真实 IP 可以用 DoH 查：`curl -s 'https://dns.google/resolve?name=HOST&type=A'`。
- 自建 / 商业服务器经常把**中继放在别的主机**（不是 ID 端口 +1）。中继连不上时问服务商，或在 `RUST_LOG=info` 日志的 `PunchHoleResponse` 行里找 `relay=`，再用 `--relay` 指定。
- 两边都在 NAT 后面时打洞很少成功，会自动走中继。

## 安全

- 设备密码是互联网和这台服务器能访问的所有端口之间唯一的屏障。请用足够长的随机密码，放在权限 600 的 env 文件里，不要写在命令行参数里。
- 设计上允许转发到整个局域网。不想这样的话，把服务器放在合适的网段，或用防火墙限制 `rustdesk-pf` 用户（`iptables -m owner --uid-owner rustdesk-pf`）只能访问指定主机和端口。
- 用专用的非特权用户运行（见上面的 systemd 部署），永远不需要 root。
- 被转发的 sshd 建议用密钥登录并设 `PasswordAuthentication no`。隧道本身是加密的，但仍应把它当成一条通往 sshd 的普通路径对待。
- API 账号密码 / token 放 env 文件，或用 `rustdesk-pf login`（token 存在状态目录）。不要提交到仓库。

## 限制

- 不支持：远程桌面、文件传输、音频、剪贴板、UDP 转发、RDP 启动器，以及 serve 端的 mux 隧道（客户端会自动退回）。
- 未实现 UDP/KCP、IPv6 和 WebRTC 打洞；已实现 TCP 打洞、同局域网直连和中继。
- 中继带宽取决于服务商；有的中继每条连接只有几十 KB/s，SSH 够用，大文件传输会很慢。

## English

`rustdesk-pf` is a headless TCP port-forwarding tool extracted from RustDesk. It speaks the same rendezvous/relay/login protocol and encryption, tries TCP hole punching / LAN direct first and falls back to the relay. `serve` registers an ID and accepts port-forward sessions (from the official client or `rustdesk-pf connect`); `connect` works like `ssh -L`. With the official client use `rustdesk --port-forward <ID> --password <PW>`: choosing "TCP tunneling" in the GUI doesn't prompt for a password until a mapping is used, and `serve` only accepts PortForward logins (a normal Connect is rejected). See the sections above for env vars, systemd / no-sudo deployment and the musl static build.

## 许可证 / License

AGPL-3.0，见 [LICENCE](LICENCE)。本项目衍生自 [RustDesk](https://github.com/rustdesk/rustdesk)（© RustDesk 作者），感谢原项目。This project is derived from RustDesk and keeps its AGPL-3.0 license.
