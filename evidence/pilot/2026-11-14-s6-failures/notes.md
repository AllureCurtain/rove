# S6 notes

## Setup flow that worked (web console)
1. Settings → 模型服务 → 新建服务: name `DeepSeek`, type OpenAI, base `https://api.deepseek.com`, default model `deepseek-flash`, API 密钥 = real key, 高级选项 → 密钥环境变量名 `DEEPSEEK_API_KEY`. Click **验证并保存** → toast `密钥已验证并安全保存`.
   - The uncontrolled password/base/model inputs need the JS `value` setter + `input`+`change` events (fill() alone doesn't register with React state reliably here).
   - Button label is **验证并保存**, not 保存更改 (memory was stale).
   - When a key is typed into 密钥, 测试连接/列出模型 are DISABLED until 验证并保存 succeeds — verify-gate.
2. Add workspace via 手动输入路径 → `/tmp/rove-pilot6-ws` → 类型 仓库 → 打开工作区.
3. Inline trust alert `需要工作区授权` → click **授权** → `provider_credentials` granted.
4. New sessions then default to `deepseek-flash` (provider_selection set by the save flow).

## Failure observations
- Bad key (deliberately wrong, not recorded here) → verify toast `密钥验证失败，请检查或重新输入密钥`; provider NOT persisted; key never in log/files/export.
- Dead base `https://127.0.0.1:1/nope` → `无法连接模型服务，请检查 API 地址与网络`.
- Bogus model on real base → `所选模型不可用，请列出模型并选择有效的模型名称` (verify actually lists models server-side).
- In-workspace missing file → `read_file` tool error `workspace file was not found`; agent corroborated w/ glob_paths and reported honestly (`blocked_missing_file`), no fabrication. Export outcome `success` (correct "cannot report").
- Out-of-workspace path `/definitely/...` → `permission_denied`, `outcome: blocked`.
- write_file approval → 拒绝 → `permission_denied`, `outcome: blocked`; workspace clean.

## Gotchas
- After a `blocked`/`failed` run, a subsequent send in the SAME session is held (stranded queue / 需关注) — message never entered transcript. Started a fresh session per scenario instead.
- Screenshots: password fields render masked; verified no key leaks in any PNG.
- `/private/tmp/...` vs `/tmp/...` canonicalization — workspace stored as `/private/tmp/rove-pilot6-ws`.
