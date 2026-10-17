# 参与贡献

感谢愿意为 zap 出力。这是一个 Windows 独占的小工具，改动请保持"毫秒级响应、毫无存在感"的手感。

## 本地开发

```bash
pnpm install
pnpm tauri dev        # 开发运行
```

提交前请确保全部通过：

```bash
pnpm build                                        # tsc + vite build
cd src-tauri && cargo test --lib                  # 单元测试
cd src-tauri && cargo fmt --check                 # 格式
cd src-tauri && cargo clippy --all-targets -- -D warnings   # lint
```

CI 对 push 与 PR 跑同一套检查，不过无法合并。

## 提交规范

- 使用约定式提交：`feat:` / `fix:` / `docs:` / `style:` / `refactor:` / `test:` / `chore:` / `ci:`
- 一个提交只做一件事；格式化、lint 清理与功能改动分开提交
- 提交信息写清"为什么"，代码本身说明"做了什么"
- 涉及 Rust 的改动请补单元测试（`#[cfg(test)]` 模块，见 `src-tauri/src/providers/vault.rs`）

## 代码约定

- Rust：每个搜索来源一个模块，统一产出 `SearchItem`；不引入新的全局可变状态（除既有的 `AtomicBool` / `Mutex` 静态量）
- 前端：组件一律走 `src/components/ui/`（shadcn/ui），不要在 `App.css` 里写会被工具类覆盖的外观属性
- 隐私红线：账号密码不得进入日志、剪贴板历史或任何上报路径；新增复制类操作必须复用 `vault_action` 的抑制与阅后即焚机制
- 动效：只做交互反馈类动画；面板呼出不做入场动画（避免闪烁），并尊重 `prefers-reduced-motion`

## 提交 PR

1. Fork 后从 `master` 切分支
2. 保持 PR 聚焦，描述改动动机与验证方式
3. UI 改动请附截图（Windows 10/11 深色桌面）

## 发布与自动更新

发布即打 tag：推送 `v*` tag（如 `v0.2.0`）后 CI 自动构建并创建公开 Release，安装包、minisign 签名（`.sig`）与 updater 清单（`latest.json`）一并上传。

版本号需要同步三处：`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`（与 tag 必须一致，CI 会校验）。

**自动更新依赖签名密钥**（minisign），已启用 `createUpdaterArtifacts`，因此：

- 仓库需配置两个 Actions secrets：`TAURI_SIGNING_PRIVATE_KEY`（私钥文件内容，本机在 `~/.tauri/zap.key`）与 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（当前密钥无密码，设为空串）。缺失时 release 构建直接失败
- 本地 `pnpm tauri build` 同样需要签名环境变量（`pnpm tauri dev` 不需要）：
  ```powershell
  $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content ~\.tauri\zap.key -Raw
  $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
  ```
- **私钥丢失 = 已装用户永远收不到更新**（公钥固定编译在客户端）。请备份 `~/.tauri/zap.key`，不要提交进仓库

用户侧更新链路：应用启动静默检查 `releases/latest/download/latest.json` → 有新版本时设置页可一键下载安装 → 重启完成更新。
