# LLBot CLI

LLBot 命令行启动器

## 功能

- 自动查找可用端口启动 PMHQ
- 终端显示登录二维码
- 保存二维码图片到 `qrcode.png`
- 进程生命周期管理（Ctrl+C 自动清理）

## 目录结构

```
llbot.exe
bin/
  pmhq/
    pmhq.exe
    pmhq_config.json
  llbot/
    node.exe
    llbot.js
```

## 命令行参数

所有参数会透传给 PMHQ，所有参数都是可选的

| 参数 | 说明 |
|------|------|
| `--qq-path=<path>` | QQ 可执行文件路径 |
| `--qq=<number>` | 快速登录 QQ 号 |
| `--headless` | 无头模式（强制终端显示二维码） |
| `--qq-console` | 启用 QQ 控制台日志 |
| `--debug` | 调试模式 |
| `--debug-pb[=true/false]` | 显示 send/recv Protobuf 日志 |
| `--work-dir=<path>` | 工作目录|
| `--sub-cmd <cmd...>` | QQ 启动后执行的子命令（必须放在最后） |
| `--sub-cmd-workdir=<path>` | 子命令工作目录（默认使用 --work-dir） |
| `--update` | 检查并执行更新 |
| `--help, -h` | 显示帮助信息 |
| `--version, -v` | 显示版本信息 |

## 使用示例

```bash
# 直接启动
./llbot

# 指定 QQ 路径
./llbot --qq-path="/opt/QQ/qq"

# 快速登录
./llbot --qq=123456789

# 无头模式
./llbot --headless

# 检查更新
./llbot --update
```

## 支持平台

- Windows x64
- Linux x64
- Linux arm64
