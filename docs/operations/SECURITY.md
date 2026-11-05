# 依赖安全告警处置

> 适用版本：**1.0.0** · 文档索引：[docs/README.md](../README.md)

本文件记录**当前所有已知的依赖告警**、处置结论与复现方式。
原则：能升级就升级，能移除就移除，**不能修的必须写清为什么不可达**，不允许静默忽略。

## 1. 三条工具的口径差异

| 工具 | 数据源 | 把「未维护」算作漏洞吗 |
|---|---|---|
| `cargo audit` | RustSec Advisory DB | 不算（记为 warning） |
| GitHub Dependabot | GitHub Advisory Database（含 RustSec 导入） | 算（按 moderate 报） |
| 本文件 | 两者交叉，按「是否可达」定性 | — |

所以经常出现「`cargo audit` 0 vulnerability，但 GitHub 报 3 个漏洞」——两者都没错，
差别在于对 unmaintained 的归类。

## 2. 当前台账

| 依赖 | Advisory | 性质 | 引入链 | 处置 |
|---|---|---|---|---|
| `ttf-parser` | RUSTSEC-2026-0192 | unmaintained（无 CVE） | 曾由 `runtime -> imageproc -> ab_glyph -> owned_ttf_parser` 引入 | **已移除**：验证码改用内嵌 5×7 点阵字模，删掉 `imageproc`/`ab_glyph`/字体文件 |
| `rustls-pemfile` | RUSTSEC-2025-0134 | unmaintained（PEM 解析已并入 rustls-pki-types） | 曾由 `api-gateway -> axum-server(tls-rustls)` 引入 | **已移除**：网关始终明文 HTTP、TLS 由外层 nginx/LB 终止，去掉 `tls-rustls` feature |
| `rsa 0.9` | RUSTSEC-2023-0071 | Marvin 时序侧信道，**上游无修复版本** | `sqlx 0.8` 声明的可选 mysql 驱动（`sqlx-mysql -> rsa`） | **不可达，保留豁免**：见下节 |

## 3. 为什么 `rsa` 消不掉

`Cargo.lock` 会记录**全部可选依赖**（与 feature 开关无关），所以只要 `sqlx` 声明了 mysql 驱动，
`sqlx-mysql -> rsa` 就一定会出现在 lock 与 Dependabot 的依赖图里。而 workspace 只启用
`postgres`：

```toml
sqlx = { version = "0.8.3", features = ["runtime-tokio", "postgres", "uuid", "chrono", "json"] }
```

验证不可达：

```bash
cargo tree -i rsa     # 输出 "nothing to print"，说明它不在实际编译的依赖图中
```

因此处理方式是：`.cargo/audit.toml` 保留豁免 + 在本文件写明原因 + 在 GitHub 上
dismiss（理由选 *Vulnerable code is not actually used*）。等异步 SQL 生态换掉 rsa
或 sqlx 拆分驱动后自然消失。

## 4. 需要 feature 才出现的告警

`telemetry` feature 会引入旧版 `tonic 0.9 -> hyper 0.14 -> h2 0.3`，命中
RUSTSEC-2026-0258（unbounded empty DATA frames）。该 feature **默认关闭**，
开启前需要评估；豁免记录同见 `.cargo/audit.toml`。

## 5. 复现与核对

```bash
# 1. 项目自带的审计（读 .cargo/audit.toml 的豁免）
cargo audit

# 2. 确认某条告警是否真的可达（能否在默认构建的依赖图里找到）
cargo tree -i <crate>

# 3. 用 OSV 按精确版本批量核对（不受 GitHub API 分页影响）
python3 - <<'PY'
import json, tomllib, urllib.request
lock = tomllib.load(open('Cargo.lock', 'rb'))
qs = [{"package": {"name": p["name"], "ecosystem": "crates.io"}, "version": p["version"]}
      for p in lock["package"] if p.get("source")]
req = urllib.request.Request("https://api.osv.dev/v1/querybatch",
                             data=json.dumps({"queries": qs[:200]}).encode(),
                             headers={"Content-Type": "application/json"})
for q, r in zip(qs, json.load(urllib.request.urlopen(req))["results"]):
    for v in r.get("vulns") or []:
        print(q["package"], q["version"], v["id"])
PY
```

## 6. 新增告警时的处理顺序

1. `cargo tree -i <crate>`：不在默认依赖图里 → 走「不可达 + 豁免 + 记录」；
2. 在依赖图里 → 先尝试升级（`cargo update -p <crate>`），再看能否移除整个依赖；
3. 都不行 → 评估实际调用路径，写清风险与本文件的台账，再 dismiss；
4. 任何豁免都必须同时出现在 `.cargo/audit.toml` **和**本文件，不允许只改一处。
