## 改动说明

<!-- 做了什么、为什么；关联 Issue 请写 Closes #123 -->

## 验证方式

- [ ] `pnpm build` 通过
- [ ] `cargo test --lib` 通过（涉及 Rust 时）
- [ ] `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` 通过（涉及 Rust 时）
- [ ] UI 改动附截图

## 隐私自检（涉及账号库/剪贴板时必填）

- [ ] 未把账号密码、剪贴板内容写入日志或任何输出
- [ ] 新增复制类操作复用 `vault_action` 的抑制剪贴板历史 / 阅后即焚机制
- [ ] 未新增任何联网或上报行为
