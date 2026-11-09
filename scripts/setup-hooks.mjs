// 让 .githooks/pre-push 的八道门禁在新 clone / 新机器上自动生效。
// core.hooksPath 只存在本机 .git/config（不入库），没有这一步时新 clone
// 默认走 .git/hooks/（仅 sample），整套本地门禁形同虚设。
import { execFileSync } from "node:child_process";

try {
  execFileSync("git", ["rev-parse", "--git-dir"], { stdio: "ignore" });
  execFileSync("git", ["config", "core.hooksPath", ".githooks"], {
    stdio: "ignore",
  });
  console.log("[setup-hooks] core.hooksPath -> .githooks");
} catch {
  // 非 git 工作区（包解包、离线构建等）不阻断安装；CI 另有服务端门禁兜底
  console.log("[setup-hooks] 非 git 工作区，跳过 hooks 配置");
}
