# Foxmail AI 增强桌面客户端开发计划

> Historical planning note. This document records an early product plan and is not
> the current project status source. For current state and handoff memory, read
> `docs/PROJECT_STATUS.md`, `docs/DECISIONS.md`, and `docs/NEXT_STEPS.md`.
>
> Important deltas from this historical plan: the current MVP skips local AI
> sensitivity auditing, stores the AI API key in SQLite plaintext by user
> decision, uses manual user-triggered remote AI analysis only, and does not aim
> to fully clone Foxmail in the first release.

## Summary

- 全新项目，不复用当前 Web/Next.js 代码。
- 第一版只做 Windows 桌面端。
- 技术栈：`Tauri v2 + Rust workspace + React/Vite + SQLite + Windows Credential Manager`。
- 开发顺序：先后端核心，再前端 UI，再测试、性能压测、打包发布。
- 产品目标：用户不再逐封人工看邮件，由本地 AI 先判敏和分流，远程强 AI 只处理安全邮件并生成摘要、分类、待办、回复建议和管理建议。

## Phase 0: Project Bootstrap

- 创建 Rust workspace + Tauri v2 + React/Vite 项目。
- 建立模块边界：
  - `crates/mail-core`
  - `crates/mail-protocol`
  - `crates/mail-store`
  - `crates/secret-store`
  - `crates/ai-guard`
  - `crates/ai-remote`
  - `crates/app-api`
  - `ui`
- 配置基础工具链：Rust stable、clippy、rustfmt、Vitest、Playwright 或 Tauri e2e。
- 建立 CI 脚本：类型检查、Rust test、前端 test、Tauri build dry run。

## Phase 1: Backend Foundation

- 在 `mail-core` 定义核心领域模型：
  - `MailAccount`
  - `MailFolder`
  - `MailMessage`
  - `AttachmentRef`
  - `SyncState`
  - `MailCommand`
  - `MailActionAudit`
  - `SensitivityReport`
  - `AiInsight`
- 在 `mail-store` 建立 SQLite migration：
  - `accounts`
  - `folders`
  - `messages`
  - `attachments`
  - `sync_states`
  - `ai_insights`
  - `ai_audits`
  - `action_audits`
- 启用 SQLite FTS5，用于邮件主题、发件人、正文、摘要搜索。
- 在 `secret-store` 接入 Windows Credential Manager。
- 明确安全规则：SQLite 不允许保存邮箱密码、OAuth refresh token、AI API key 明文。

## Phase 2: Mail Engine

- 在 `mail-protocol` 实现 IMAP/SMTP 手动账号支持。
- 第一版优先支持：
  - 添加邮箱账户
  - 测试 IMAP 连接
  - 测试 SMTP 连接
  - 拉取文件夹
  - 增量同步 INBOX
  - 拉取邮件正文
  - 发送邮件
- 在 `mail-core` 实现同步状态机：
  - `idle`
  - `syncing`
  - `watching`
  - `backoff`
  - `error`
  - `disabled`
- 每个账户/文件夹必须有同步锁，防止重复同步。
- 同步策略：
  - 应用启动自动同步启用账户。
  - 运行期间定时增量同步。
  - 支持 IMAP IDLE 的服务商进入监听模式。
  - 失败后指数退避。
- 所有邮件操作通过 command bus：
  - `mark_read`
  - `mark_unread`
  - `star`
  - `unstar`
  - `move`
  - `archive`
  - `delete`
  - `send`
- 高风险动作必须进入确认队列：
  - `send`
  - `permanent_delete`
  - `batch_delete`
  - `batch_move`
  - `forward`

## Phase 3: AI Safety Gate

- 在 `ai-guard` 先实现规则引擎版本，本地模型适配层后续替换。
- `SensitivityReport` 输出字段：
  - `level`: `safe | suspicious | sensitive | blocked`
  - `confidence`
  - `reasons`
  - `matched_rules`
  - `allowed_remote_fields`
  - `created_at`
- 本地判敏输入：
  - 发件人
  - 收件人
  - 主题
  - 正文片段
  - 附件文件名、大小、MIME
  - 邮件头部关键字段
- 判敏规则第一版覆盖：
  - 身份证、护照、银行卡、手机号
  - 合同、发票、工资、财务、法务、医疗关键词
  - 密码、验证码、token、secret、key
  - 附件中可疑类型
  - 外部域名和钓鱼特征
- `safe` 才允许进入远程 AI。
- `suspicious` 默认只允许脱敏摘要，或等待用户确认。
- `sensitive` 和 `blocked` 禁止远程上传。

## Phase 4: Remote AI Pipeline

- 在 `ai-remote` 实现远程 AI provider 抽象。
- 远程 AI 能力第一版包括：
  - 邮件摘要
  - 优先级判断
  - 待办提取
  - 分类标签
  - 回复草稿
  - 批量整理建议
- 上传前必须经过 `ai-guard`。
- 每次远程请求必须写审计：
  - `message_id`
  - `sensitivity_level`
  - `uploaded_fields`
  - `redaction_applied`
  - `model`
  - `request_time`
  - `result_id`
- 远程 AI 返回结果先保存为建议，不直接执行危险动作。
- AI 自动动作权限：
  - 可自动：打标签、生成摘要、标记优先级、加入待办建议。
  - 需确认：发送、删除、转发、批量移动、批量归档。

## Phase 5: App API

- 在 `app-api` 暴露 Tauri commands。
- 前端只调用 commands，不直接读数据库、不直接碰密钥。
- 必要 commands：
  - `add_account`
  - `test_account_connection`
  - `list_accounts`
  - `sync_account`
  - `get_sync_status`
  - `list_messages`
  - `get_message`
  - `search_messages`
  - `execute_mail_action`
  - `list_ai_insights`
  - `run_ai_analysis`
  - `list_pending_actions`
  - `confirm_action`
  - `reject_action`
  - `get_audit_log`
- 所有 command DTO 使用明确结构，禁止把数据库内部模型直接暴露给前端。

## Phase 6: Frontend

- UI 在后端稳定后开始。
- 视觉方向：冷硬、低饱和、终端感、信息密度高，避免花哨动效。
- 页面结构：
  - 左侧账户/文件夹栏
  - 中间聚合邮件列表
  - 右侧邮件详情和 AI 面板
  - 底部或侧边同步状态
  - 独立设置页
  - AI 待确认动作队列
- 第一版 UI 功能：
  - 添加邮箱账户
  - 查看聚合收件箱
  - 搜索邮件
  - 查看邮件详情
  - 查看 AI 摘要、风险等级、待办
  - 确认或拒绝 AI 建议动作
  - 查看同步状态和错误
- 前端不做复杂业务逻辑，只做展示、筛选、用户确认。

## Phase 7: Testing

- Rust 单元测试：
  - 同步状态机
  - SQLite repository
  - secret-store
  - 判敏规则
  - 脱敏逻辑
  - command bus 权限
- 集成测试：
  - mock IMAP server
  - mock SMTP server
  - 账户添加和连接测试
  - 增量同步
  - 失败重试
  - 发送邮件
- AI 安全测试：
  - `safe` 邮件允许远程 AI
  - `suspicious` 邮件默认脱敏或等待确认
  - `sensitive` 邮件不触发远程请求
  - `blocked` 邮件不触发远程请求
  - 审计日志完整落库
- 性能测试：
  - 10k 邮件列表加载
  - 100k 邮件 FTS 搜索
  - 大附件索引
  - 多账户并发同步
  - UI 不被同步任务阻塞
- 桌面测试：
  - 应用启动自动同步
  - 最小化托盘运行
  - 关闭应用停止同步
  - 断网恢复
  - Windows 通知
  - 打包后安装运行

## Phase 8: Packaging

- 使用 Tauri 生成 Windows 安装包。
- 配置应用签名预留接口。
- 配置自动更新预留接口，但第一版可不启用。
- 打包前检查：
  - 数据库 migration 可重复执行
  - Credential Manager 正常读写
  - 无明文密钥落盘
  - 远程 AI key 不出现在日志
  - 敏感邮件不会上传
  - 崩溃后同步状态可恢复

## Milestones

- M1：项目骨架 + SQLite + Credential Manager。
- M2：单邮箱 IMAP/SMTP 添加、同步、查看、发送。
- M3：多账户聚合、同步状态机、搜索、审计日志。
- M4：本地 AI 判敏门禁。
- M5：远程 AI 摘要、分类、待办、回复草稿。
- M6：React UI 对接完整后端能力。
- M7：安全测试、性能测试、Windows 打包。

## Non-Negotiable Rules

- 不保存明文邮箱密码、OAuth refresh token、AI API key。
- 前端不直接访问邮箱协议、不直接访问密钥、不直接访问数据库文件。
- 远程 AI 请求必须经过本地判敏。
- 敏感邮件默认不上传。
- AI 不允许默认发送邮件或永久删除邮件。
- 所有 AI 判断和邮件操作必须可审计。
- 后端优先，UI 后置。
- 性能和安全优先于视觉复杂度。

## First Coding Order

1. 初始化 Tauri + Rust workspace + React/Vite。
2. 建立 SQLite migration 系统。
3. 建立 Credential Manager 封装。
4. 定义 mail-core 领域模型。
5. 实现 mail-store repository。
6. 实现 IMAP 连接测试和文件夹读取。
7. 实现 INBOX 增量同步。
8. 实现 SMTP 发送。
9. 实现 command bus 和审计日志。
10. 实现本地判敏规则引擎。
11. 实现远程 AI mock provider。
12. 实现 Tauri commands。
13. 开始 React UI。
14. 做集成测试、性能测试和打包。
