# 给 AI agent 的被控端配置提示词

把下面代码块里的整段文字原样复制给目标机器上的 AI agent（智能体，能执行命令的 AI 助手），它会从零装好 `rustdesk-pf` 被控端（serve，被远程连接的一方）：先尝试打洞（hole punch，两端直接建立连接），失败时自动走中继（relay，由中转服务器转发流量）。提示词里的密码都是占位符，agent 会向你索要，**不要把真实密码直接贴进提示词**。

```text
你的任务：在这台机器上从零安装并配置 rustdesk-pf 被控端（serve），让它常驻运行、开机自启。连接方式是先 TCP 打洞（hole punch，两端直连），不通时自动走中继（relay，中转服务器转发）。请按以下步骤操作，每一步都检查结果。

【1. 判断架构】
- 运行 `uname -m` 和 `uname -s`。
- 只有 x86_64 Linux 才用预编译的 musl 包（musl：一种 C 库，这里指静态链接、不依赖系统 glibc/libssl 的二进制）。
- 其他架构（如 aarch64）或系统：没有预编译包，需要装 Rust 工具链后自行编译：
  git clone -b pf-cli https://github.com/Cross2pro/rustdesk-pf && cd rustdesk-pf && cargo build -p rustdesk-pf --release
  产物在 target/release/rustdesk-pf；deploy/ 目录就用仓库里的。

【2. 下载、校验、安装（x86_64 Linux）】
- 从 https://github.com/Cross2pro/rustdesk-pf/releases/latest 下载 `rustdesk-pf-v<版本>-x86_64-linux-musl.tar.gz` 和 `sha256sums.txt`（可用 `gh release download -R Cross2pro/rustdesk-pf`，或在网页上找到附件链接用 curl -LO 下载）。
- 校验：`sha256sum -c sha256sums.txt --ignore-missing`，必须显示 OK，否则停止并报告。
- 解压，包里有 rustdesk-pf 二进制、README.md、LICENSE、deploy/。
- 安装：有 sudo 就 `sudo install -m755 rustdesk-pf /usr/local/bin/`；没有 sudo 就 `install -D -m755 rustdesk-pf ~/.local/bin/rustdesk-pf`。
- 运行 `rustdesk-pf --version` 确认。

【3. 写 env 配置文件（权限 600）】
- 有 sudo：/etc/rustdesk-pf/rustdesk-pf.env（目录 `sudo install -d -m700 /etc/rustdesk-pf`）。
- 无 sudo：~/.config/rustdesk-pf/rustdesk-pf.env。
- 先 `install -m600 /dev/null <文件>` 建空文件再编辑，保证从一开始就是 600。内容：
  RUSTDESK_PF_SERVER=47.96.135.20:60016
  RUSTDESK_PF_RELAY=106.55.21.51:60027
  RUSTDESK_PF_KEY=k3lsu+CTLs4OhFpq5Lh38Uvo2m8Cyb1jLz6gTCAnyCw=
  RUSTDESK_PF_API=https://106.55.21.51:41112
  RUSTDESK_PF_API_HOST=api.nemocc.top
  RUSTDESK_PF_ACCOUNT_USER=<账户>
  RUSTDESK_PF_ACCOUNT_PASSWORD=<账户密码>
  RUSTDESK_PF_PASSWORD=<设备密码，长随机>
- 尖括号里是占位符：账户和账户密码向人要；设备密码（控制端连接时要输入的密码）可以向人要，或用 `openssl rand -base64 24` 生成一个长随机密码，并告诉人它保存在哪个文件里。
- 密码绝不能写进日志、Git 仓库、shell 历史或命令行参数（命令行参数会被 ps 看到）；只放在这个 env 文件里，通过编辑器或 heredoc 写入，不要 echo 到终端。
- 必须用上面的真实 IP，不要换成域名：有些网络用 fake-ip DNS（假 IP 解析，如 Clash/sing-box TUN），会把域名解析到 198.18.x.x，导致连接失败。API 用 IP，再用 RUSTDESK_PF_API_HOST 带上域名作 Host 头。

【4. 登录并前台试跑】
- 状态目录（RUSTDESK_PF_CONFIG，存 ID、密钥、token）必须和常驻服务用的一致：
  - systemd 方式：/var/lib/rustdesk-pf（以 rustdesk-pf 用户运行，见第 5 步，先建用户）；
  - 免 sudo 方式：~/.config/rustdesk-pf/state。
- 加载 env 后登录（login：用账户换取 access_token 访问令牌，服务器要求登录后才允许打洞）：
  set -a; . <env 文件>; set +a; export RUSTDESK_PF_CONFIG=<状态目录>
  rustdesk-pf login
  systemd 方式可由 root 执行，环境变量只在子 shell 里，不会出现在命令行：
  sudo install -d -m700 /var/lib/rustdesk-pf
  sudo sh -c 'set -a; . /etc/rustdesk-pf/rustdesk-pf.env; set +a; export RUSTDESK_PF_CONFIG=/var/lib/rustdesk-pf; rustdesk-pf login && rustdesk-pf serve'
  试跑结束后（第 5 步建好 rustdesk-pf 用户后）执行 `sudo chown -R rustdesk-pf:rustdesk-pf /var/lib/rustdesk-pf`，让服务用户能读写 ID 和 token。
- 前台试跑 `rustdesk-pf serve`，等日志出现 `RegisterPk OK; id=<ID> online`，记下这个 ID。若出现 `UUID_MISMATCH; regenerating id`，ID 会被重新生成，以最后一次 `RegisterPk OK` 的 ID 为准。确认后 Ctrl+C 停止。`rustdesk-pf id` 也能打印当前 ID。

【5. 设为常驻、开机自启】（以解压包或仓库里的 deploy/ 为准）
- 有 sudo（推荐 systemd）：
  sudo useradd --system --home /var/lib/rustdesk-pf --shell /usr/sbin/nologin rustdesk-pf
  sudo install -m644 deploy/rustdesk-pf.service /etc/systemd/system/
  sudo systemctl daemon-reload && sudo systemctl enable --now rustdesk-pf
  （unit 里 EnvironmentFile=/etc/rustdesk-pf/rustdesk-pf.env、RUSTDESK_PF_CONFIG=/var/lib/rustdesk-pf、ExecStart=/usr/local/bin/rustdesk-pf serve；状态目录归属需是 rustdesk-pf 用户。）
- 无 sudo：脚本默认二进制在 ~/bin/rustdesk-pf，装在 ~/.local/bin 时要设 RUSTDESK_PF_BIN：
  RUSTDESK_PF_BIN=~/.local/bin/rustdesk-pf sh deploy/rustdesk-pf-user.sh restart   # 还支持 start|stop|status|log
  开机自启：有 systemd --user 就把 deploy/rustdesk-pf.service 复制到 ~/.config/systemd/user/，删掉 User=/Group= 以及 ProtectHome 等加固行，EnvironmentFile 改成 ~/.config/rustdesk-pf/rustdesk-pf.env 的绝对路径，RUSTDESK_PF_CONFIG 改成 ~/.config/rustdesk-pf/state 的绝对路径，ExecStart 改成二进制绝对路径，然后 `systemctl --user enable --now rustdesk-pf` 并 `loginctl enable-linger $USER`（退出登录后继续运行）；没有 systemd --user 就用 crontab 加 `@reboot RUSTDESK_PF_BIN=<绝对路径> sh <脚本绝对路径> start`。

【6. 自检】
- 出站连通：`nc -vz 47.96.135.20 60016`、`nc -vz 106.55.21.51 60027`、`nc -vz 106.55.21.51 41112`（或用 curl / bash 的 /dev/tcp）。ID 端口 60016 还需要 UDP 出站（注册走 UDP）。
- 运行状态：`systemctl status rustdesk-pf` 和 `journalctl -u rustdesk-pf -n 50`；免 sudo 方式看 `sh deploy/rustdesk-pf-user.sh status` 和 ~/.config/rustdesk-pf/serve.log。日志里应持续有 RegisterPk OK，且没有 "please log in" 之类错误。
- 重启服务后 ID 应保持不变。

【7. 安全】
- 不要开放任何入站端口、不要改防火墙放行入站、不要开 UPnP；rustdesk-pf 只需出站连接。
- 设备密码要强（建议 20 位以上随机）；env 文件保持 600，状态目录 700。

【8. 回报】
完成后回报：
- 最终 ID；
- 设备密码保存在哪个文件（不要回显密码本身）；
- 运行方式（systemd / systemd --user / 脚本 + cron）以及查看日志的命令；
- 控制端连接命令示例：
  rustdesk-pf connect <ID> -L 127.0.0.1:2222:127.0.0.1:22
  （控制端需配置同样的 RUSTDESK_PF_SERVER/RELAY/KEY/API/API_HOST/账户变量，并用 RUSTDESK_PF_PEER_PASSWORD 提供设备密码；可选加 --force-relay 强制走中继。连上后 `ssh -p 2222 <user>@127.0.0.1`。）
- 任何失败的步骤和错误日志（去掉密码/token）。
```
