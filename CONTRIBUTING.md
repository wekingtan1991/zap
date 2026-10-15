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
