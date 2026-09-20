# Spacecraft Telemetry FFI Toolkit

离线 CCSDS 遥测传输帧（TM Transfer Frame, CCSDS 132.0-B）解析与故障推理工具。
帧结构通过外部 **JSON** 描述动态加载，故障树规则用 **YAML** 描述，输出 ANSI
彩色终端报告。

## 功能

- **协议**：完整解析 TM 传输帧主帧头（版本号/航天器 ID/VCID/帧计数/同步标志/
  FHP）、可变长副帧头（含版本与长度字段）、M_PDU 多路复用数据域、可选 OCF/CLCW。
- **位级解析**：字段偏移 + 位宽驱动，支持枚举映射、有符号数、线性标定
  （`scale`/`offset`/`unit`）、多进制显示。
- **空间包重组**：按虚拟通道缓存跨帧分段，解析 CCSDS 空间包主帧头与按 APID
  配置的包副帧头，自动跳过空闲包（APID 0x7FF）与填充字节。
- **故障推理**：YAML 故障树支持 `and`/`or`/`not`/`at_least` 门逻辑与表达式叶子
  （`== != < <= > >=`、`and/or/not`、括号），按分系统聚合并给出总体状态、
  判定证据链与处置建议。
- **子命令**：
  - `parse` — 解析并打印字段与空间包
  - `infer` — 解析并执行故障树推理
  - `diff` — 两帧字段级对比（支持同一文件内两帧或两个文件）
  - `gen-sample` — 生成标称/异常示例帧，便于离线体验
- 彩色报告遵循 `NO_COLOR`，并支持 `--color`/`--no-color` 与 `--json` 机器可读输出。

## 工作区结构

| Crate | 职责 |
| --- | --- |
| `crates/frame_parser` | JSON 帧结构定义、位级解码、M_PDU 空间包重组、示例帧构建器 |
| `crates/rule_engine` | YAML 规则集、表达式求值、故障树推理与分系统聚合 |
| `crates/report_renderer` | ANSI 着色的 parse/infer/diff 报告 |
| `crates/cli` | `clap` 驱动的 `tmctl` 命令行 |

## 快速开始

```bash
cargo run -p tmctl -- gen-sample
cargo run -p tmctl -- parse examples/generated/nominal.tm
cargo run -p tmctl -- infer examples/generated/anomaly.tm
cargo run -p tmctl -- diff  examples/generated/stream.tm
```

`stream.tm` 内含两帧（标称 + 异常），`diff` 默认比较其中第 0、1 帧；
也可分别指定文件：

```bash
cargo run -p tmctl -- diff nominal.tm --right anomaly.tm
```

常用参数：`-s/--spec` 指向 JSON 帧定义，`-r/--rules` 指向 YAML 规则，
`--json` 输出 JSON，`--limit N` 只处理前 N 帧，`--no-color` 关闭颜色。

## JSON 帧结构定义

参见 `examples/tm_frame_spec.json`。核心片段：

```json
{
  "frame_length_octets": 1115,
  "asm": "1ACFFC1D",
  "sync_flag_bit": 32,
  "fhp_bit": 37,
  "primary_header": {
    "length_octets": 6,
    "fields": [
      { "name": "spacecraft_id", "offset": 2, "bits": 10 },
      { "name": "vcid", "offset": 12, "bits": 3,
        "enum_values": { "0": "VC0_REALTIME", "7": "VC_IDLE" } }
    ]
  },
  "secondary_header": { "presence_flag_bit": 15, "length_bit": 4,
    "length_bits": 10, "length_mode": "data_minus_one", "fields": [ ... ] },
  "packet": { "idle_apid": 2047, "primary_header": [ ... ],
              "secondary_by_apid": { "26": { "label": "eps_hk", ... } } }
}
```

字段支持 `signed`、`scale`、`value_offset`、`unit`、`radix`、`enum_values`，
副帧头字段可用 `anchor`（`region_start`/`frame_start`/`data_field_start`）选择
偏移参考点。枚举键接受十进制或 `0x` 十六进制字符串。

## YAML 故障树

参见 `examples/fault_rules.yaml`：

```yaml
- id: EPS_BATT_UNDERVOLTAGE
  subsystem: EPS
  title: Battery bus undervoltage
  severity: critical
  recommendation: Shed non-essential loads.
  tree:
    gate: or
    children:
      - expr: sh_batt_voltage < 24
      - expr: pkt_eps_hk_bus_voltage < 24
```

字段名即解析产物的键：主帧头字段（`spacecraft_id`…）、副帧头字段（`sh_*`）、
CLCW（`clcw_*`）、空间包字段（`pkt_<label>_<field>`）。带枚举映射的字段按字符串
比较（如 `sh_eps_mode == "EPS_SUN"`），其余按数值比较；可选字段缺失（例如未带
OCF 时引用 `clcw_*`）按“条件不成立”处理，便于编写跨配置规则。

## 测试

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
```
