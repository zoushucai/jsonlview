# jlv

用于查看 `JSONL` 文件的命令行工具，默认按流式方式读取

## 功能

- `head -n` 查看前 `n` 行
- `tail -n` 查看后 `n` 行
- `range start num` 查看从第 `start` 行开始的连续 `num` 行
- `random [num]` 快速近似随机抽取 `num` 行
- `random-buf [num]` 使用全文件蓄水池抽样随机抽取 `num` 行
- `-c, --count` 统计总行数
- `-p, --pretty [num]` 以格式化 JSON 输出，并控制条目之间插入多少个空行
- `--max` 对过长字符串做截断，只保留前若干字符，后面显示 `...`
- `-l, --line` 控制是否显示 `[line N]`

## 构建

```powershell
cargo build --release
```

生成的可执行文件：

- `target\release\jlv.exe`

## 安装

### 从 GitHub Release 下载

按你的平台下载对应文件即可。

### Windows

1. 下载 `jlv-windows-x86_64.exe`
2. 重命名为 `jlv.exe`（可选）
3. 放到你常用目录，例如：

```powershell
C:\Tools\jlv\jlv.exe
```

4. 直接运行：

```powershell
.\jlv.exe head data.jsonl
```

5. 如果希望全局可用，把它所在目录加入 `PATH`，之后可以直接运行：

```powershell
jlv head data.jsonl
```

### Linux (Ubuntu)

1. 下载 `jlv-linux-x86_64`
2. 给执行权限：

```bash
chmod +x jlv-linux-x86_64
```

3. 可选：重命名并放到系统路径：

```bash
mv jlv-linux-x86_64 jlv
sudo mv jlv /usr/local/bin/
```

4. 之后直接运行：

```bash
jlv head data.jsonl
```

### macOS

当前 release 提供 Apple Silicon 版本：

- `jlv-macos-aarch64`

给执行权限：

```bash
chmod +x jlv-macos-aarch64
```

可选：重命名并放到系统路径：

```bash
mv jlv-macos-aarch64 jlv
sudo mv jlv /usr/local/bin/
```

之后直接运行：

```bash
jlv head data.jsonl
```

如果 macOS 首次提示“无法验证开发者”，可以在“系统设置 -> 隐私与安全性”里允许执行，或手动移除隔离属性：

```bash
xattr -d com.apple.quarantine jlv
```

### 从源码构建

如果你不想下载 release，也可以本地构建：

```bash
cargo build --release
```

生成文件：

- Windows: `target\release\jlv.exe`
- Linux/macOS: `target/release/jlv`

## 用法

```powershell
jlv head "example\data.jsonl"
jlv head 5 "example\data.jsonl"
jlv head -n 8 "example\data.jsonl"
jlv head "example\data.jsonl" -p

jlv tail "example\data.jsonl"
jlv tail 8 "example\data.jsonl"
jlv tail --num 8 -l "example\data.jsonl"

jlv range "example\data.jsonl"
jlv range 10 20 "example\data.jsonl"
jlv range --start 10 --num 20 -p 1 -m 40 -l "example\data.jsonl"

jlv random "example\data.jsonl"
jlv random 3 "example\data.jsonl"
jlv random-buf -n 3 -p 2 -m 40 "example\data.jsonl"

jlv -c "example\data.jsonl"
jlv --count --file "example\data.jsonl"
```

## 说明

- `--file` 可以缺省，文件路径也可以直接作为裸参数传入
- `head` / `tail` / `random` / `random-buf` 默认 `num=5`
- `range` 默认 `start=0`、`num=5`
- `range` 的 `start` 使用从 `0` 开始的偏移量
- `tail` 为了适配大文件，采用从文件末尾反向扫描的方式
- `random` 默认使用快速近似抽样，优先速度
- `random-buf` 使用全文件蓄水池抽样，结果更均匀，但需要扫描整个文件
- `-p/--pretty` 不带值时等价于 `-p 0`
- `-p/--pretty num` 支持任意非负整数，表示条目之间插入 `num` 个空行
- `--max 0` 表示格式化输出时不截断字符串
- `-l/--line` 开启后显示 `[line N]`