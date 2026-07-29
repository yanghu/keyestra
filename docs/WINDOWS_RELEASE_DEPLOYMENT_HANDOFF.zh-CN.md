# Windows Release 部署交接文档

## 目标

将 Cargo 的构建输出与 Windows 托盘程序实际运行的二进制文件分离。

期望的工作流必须允许我们在旧版本仍然运行时构建并暂存新 release。
Windows 可以锁定正在运行的 `.exe`，但这种锁定绝不能再阻止
`target\release` 中的构建。

这份交接文档的范围仅限 release 部署和 Startup 管理。实现过程中不要顺带
重新设计 MIDI 映射、recorder 行为或 monitor UI。

## 交接时已确认的状态

- 仓库路径：

  ```text
  C:\Users\hueyh\OneDrive\Documents\midi curve
  ```

- 当前 commit：`9a9460c`（`Show monitor build version in dashboard`）。
- `target\release` 已在 2026 年 7 月 28 日基于这个 commit 成功重新构建。
- Monitor 页面显示的构建标识为 `v0.1.0 · 9a9460c0`。
- `target\manual-release` 已经删除。
- 重新构建和验证标准 release 时，所有 FP10 进程均已退出。之后用户又在
  2026 年 7 月 28 日晚上约 10:09 从 `target\release` 启动了三个二进制，
  因此撰写本文档时该目录再次处于被锁定状态。
- 当前 Startup 脚本位于：

  ```text
  %APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\fp10-map-tray.vbs
  ```

- 交接时，该脚本直接指向：

  ```text
  C:\Users\hueyh\OneDrive\Documents\midi curve\target\release\fp10-map-tray.exe
  ```

- Tray 会在自身所在目录寻找同级的 `fp10-map.exe` 和
  `fp10-monitor-server.exe`。必须保留这个约束。
- Tray 的 `Install startup` 操作目前会将 `env::current_exe()` 写入 Startup
  VBS。代码位于 `src/bin/fp10-map-tray.rs` 的 `install_startup()`。
- 用户数据目前位于 `%APPDATA%\fp10-map`。部署过程不得移动或删除其中的
  设置、日志和录音文件。

## 当前目录结构为什么不正确

`target\release` 是 Cargo 的输出目录，不是安装目录。直接运行
`target\release\fp10-map-tray.exe` 后，Windows 会锁定这个文件以及同目录的
mapper 和 monitor 二进制。下一次执行 `cargo build --release` 时，Cargo
便无法替换这些被锁定的文件。

每次构建前停止 live 程序虽然能绕过这个问题，但会带来以下后果：

- 中断 MIDI 转发和 monitor；
- 清除 recorder 位于内存中的滚动缓存；
- 使自动化 release 验证变得脆弱；
- 混淆“构建产物”和“已部署程序”这两个概念。

不要通过创建 `target\manual-release` 之类的另一个 Cargo target 目录解决
这个问题。那只会制造另一个含义不清的构建位置。

## 推荐的部署目录结构

Cargo 输出保持不变：

```text
<workspace>\target\release\
  fp10-map.exe
  fp10-map-tray.exe
  fp10-monitor-server.exe
```

将不可变、带版本号的副本部署到仓库之外：

```text
%LOCALAPPDATA%\fp10-map\
  releases\
    0.1.0-9a9460c0\
      fp10-map.exe
      fp10-map-tray.exe
      fp10-monitor-server.exe
      examples\
        curve.toml
        curve-mid-control.toml
  current.json
```

已安装的程序文件使用 `%LOCALAPPDATA%`；现有的 roaming 用户数据继续使用
`%APPDATA%`。

每个 release 目录发布后都视为不可变。新版本复制到新的目录，因此正在
运行的旧版本永远不会锁住构建输出或新版本的部署位置。

建议保留 `current.json`，供诊断和未来工具读取。最小结构如下：

```json
{
  "version": "0.1.0",
  "build": "9a9460c0",
  "path": "C:\\Users\\...\\AppData\\Local\\fp10-map\\releases\\0.1.0-9a9460c0"
}
```

第一版不要依赖目录符号链接或 junction。它们会增加权限、OneDrive 和原子
切换方面的复杂度。

## 推荐的第一版实现

在仓库中新增：

```text
scripts\deploy-windows.ps1
```

脚本应执行以下步骤：

1. 将 workspace、Cargo 输出目录、部署根目录和 Startup 路径全部解析为
   绝对路径。
2. 默认拒绝在 Git 工作区有未提交改动时部署。可以提供明确的开发模式
   override，但部署后的构建标识必须保留 `-dirty` 后缀。
3. 运行：

   ```powershell
   cargo fmt --check
   cargo test
   cargo build --release --bin fp10-map --bin fp10-map-tray --bin fp10-monitor-server
   ```

4. 从 `Cargo.toml` 获取包版本，并通过
   `git rev-parse --short=8 HEAD` 获取 build ID。
5. 在以下目录下创建名称唯一的 staging 目录：

   ```text
   %LOCALAPPDATA%\fp10-map\releases
   ```

6. 将三个 release 二进制和两个内置 curve 文件复制到 staging 目录。
   三个二进制的同级布局和名称必须保持不变。
7. 验证所有必需文件都存在且大小不为零。
8. 将 staging 目录重命名为最终的不可变版本目录。staging 与最终路径必须
   位于同一个磁盘卷上。
9. 先写入一个同级临时文件，再通过 rename 原子更新 `current.json`。
10. 调用新部署的 tray 更新 Startup：

    ```powershell
    & "$releaseDir\fp10-map-tray.exe" --install-startup
    ```

    因为 `install_startup()` 使用 `env::current_exe()`，这样 VBS 会直接
    指向这个新部署的版本，也不需要在部署脚本中重复实现 VBS 转义。
11. 输出部署后的 tray 路径、Startup 脚本路径、版本、build ID，以及是否
    尚待激活。

默认部署过程不得停止或替换当前正在运行的版本。用户练琴或 recorder 中存在
未保存缓存时，也必须能够安全部署。

## 激活语义

将“部署”和“激活”视为两个不同操作：

- **部署：**构建、复制到新的不可变目录，并更新下次登录使用的 Startup。
  不得中断当前进程。
- **立即激活：**退出旧 tray，启动新部署的 tray。这个操作会清除 monitor
  位于内存中的 recorder 缓存，因此必须由用户明确触发。

可以提供可选的 `-Activate` 参数，但它必须：

1. 明确警告未保存的滚动 MIDI 缓存将丢失。
2. 只停止 `fp10-map`、`fp10-map-tray` 和
   `fp10-monitor-server`。
3. 不得终止其他进程，也不得使用范围过大的路径或 glob 删除操作。
4. 启动已部署的 tray，而不是 `target\release` 中的 tray。
5. 验证 tray 和 monitor 启动后仍在运行。

正常部署和 Startup 安装不应要求管理员权限。建议的所有路径都是当前用户
路径。如果旧 tray 曾以管理员身份运行，立即激活时可能需要用户手动退出它。

## Startup 与回滚

Startup VBS 可以直接指向选中的不可变版本目录。每次成功部署都会将它更新为
新版本。

回滚操作应保持简单：

```powershell
& "$oldReleaseDir\fp10-map-tray.exe" --install-startup
```

如果希望立即回滚，再退出当前 tray 并启动旧版本的已部署 tray。

部署时不要删除上一个已部署版本。至少保留最近两个成功版本。

清理旧版本必须作为独立且保守的操作：

- 不得删除 Startup 当前指向的目录；
- 不得删除 `current.json` 当前指向的目录；
- 不得删除包含正在运行的 FP10 二进制的目录；
- 如果由于权限原因无法检查正在运行的可执行文件路径，应跳过清理，不得猜测。

## Curve 与设置迁移注意事项

`curve_path()` 会首先寻找：

```text
<tray directory>\examples\<curve file>
```

然后才回退到相对于 workspace 的路径。因此已部署的包必须包含 `examples`
目录。

`tray-settings.toml` 当前保存的是 curve 的绝对路径。已有设置可能仍然指向
workspace，第一版实现必须测试这个迁移场景。

期望行为：

- 内置的 Forum 和 Mid-control 选项解析到已部署目录内的 `examples` 文件；
- 真正的自定义 curve 继续保留用户设置的绝对路径；
- 部署过程不得覆盖用户的自定义 curve。

必要时，可修改设置序列化方式：内置选项保存独立的 choice 标识，自定义
curve 才保存路径。README 与测试必须同步更新。

## 可能需要修改的文件

- 新增：`scripts/deploy-windows.ps1`
- `README.md`
  - 说明 build、deploy 和 activate 的区别；
  - 不再指导用户将 `target\release` 作为长期运行程序；
  - 说明已安装二进制和回滚版本的位置。
- `AGENTS.md`
  - 删除“release 二进制通常从 `target\release` 运行”的假设；
  - 按版本化部署方式更新构建、部署和重启说明。
- 可能需要修改 `src/bin/fp10-map-tray.rs`
  - curve 设置迁移；
  - 可选的已部署版本/状态显示；
  - 只有当部署脚本无法安全复用 `--install-startup` 时，才修改 Startup
    行为。
- 为所有改变的 Rust 行为补充测试。

除非用同样明确的打包布局替代，否则不要移除同级二进制查找机制。

## 验证要求

运行项目的常规验证：

```powershell
cargo fmt --check
cargo test
cargo build --release --bin fp10-map --bin fp10-map-tray --bin fp10-monitor-server
```

然后验证部署工作流：

1. 部署版本 A。
2. 从 `%LOCALAPPDATA%` 启动版本 A。
3. 确认 Startup 指向版本 A，而不是仓库。
4. 在版本 A 仍运行时修改或构建版本 B。
5. 确认无需停止版本 A 即可成功执行 `cargo build --release`。
6. 在版本 A 仍运行时部署版本 B。
7. 确认版本 A 继续运行，版本 B 文件完整。
8. 确认 Startup 已改为指向版本 B。
9. 明确执行版本 B 的立即激活。
10. 分别从电脑和手机打开 monitor。
11. 确认页面底部显示版本 B 的 build ID。
12. 确认 Overview、`全部`、`回到最新`、拖动/平移、缩放按钮、桌面
    Ctrl/Cmd + 滚轮和手机 pinch 均存在。
13. 将 Startup 回滚到版本 A，并验证回滚。
14. 确认 `%APPDATA%\fp10-map` 中的录音、设置和日志均未受影响。

没有真实 MIDI 硬件时无法完整验证硬件行为。若缺少硬件，应明确说明没有手动
测试 live MIDI 重连和 recorder 行为。

## 验收标准

- 运行已安装的 release 时，不会锁住 `target` 下的任何文件。
- 旧版本仍运行时，可以构建并部署新版本。
- Startup 始终指向不可变的已部署 release，而不是 Cargo target 目录。
- 部署为当前用户级别，正常情况下无需管理员权限。
- 部署不会静默重启程序或清除 recorder 内存。
- 立即激活必须是明确操作。
- Monitor footer 能标识准确的已部署 build。
- 至少保留一个旧 release 用于回滚。
- 部署失败后，临时和 staging 目录能被安全清理。
- README 与 `AGENTS.md` 准确描述新工作流。

## 第一版不包含的范围

- 自动后台更新。
- 从 GitHub 下载 release。
- 系统级安装程序。
- 安装为 Windows Service。
- 原地自更新正在运行的可执行文件。
- 移动或重新设计 recorder 持久化。
