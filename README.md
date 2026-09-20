# ccsds-tm — CCSDS 遥测帧离线解析与故障推理工具

面向卫星地面站的离线分析工具：解析 CCSDS TM 下行帧（主帧头 / 副帧头 / 数据域），
按外部 JSON 动态描述帧结构，再依据 YAML 故障树规则推理各分系统健康状态，
输出 ANSI 彩色报告。

## 模块结构

| 模块 | 职责 |
| --- | --- |
| `src/frame_parser/` | CCSDS 主帧头解析、JSON 帧结构定义加载、MSB 优先位提取 |
| `src/rule_engine/` | YAML 故障树规则（all/any/not 组合）求值、分系统状态汇总 |
| `src/report_renderer/` | ANSI 彩色报告渲染（parse / infer / diff） |
| `src/cli.rs` | clap 子命令入口 |

## 子命令

```bash
cargo build --release

# 解析单帧
./target/release/ccsds-tm parse -f assets/frame_b.bin -d assets/tm_frame_def.json

# 故障推理
./target/release/ccsds-tm infer -f assets/frame_b.bin -d assets/tm_frame_def.json -r assets/fault_rules.yaml

# 对比两帧差异
./target/release/ccsds-tm diff -d assets/tm_frame_def.json assets/frame_a.bin assets/frame_b.bin
```

## 帧结构定义（JSON）

`assets/tm_frame_def.json` 描述帧总长、副帧头与数据域的字段布局：

- `byte_offset` / `bit_offset` / `bit_width`：字段位置（位序为 MSB 优先，符合 CCSDS 约定）
- `type`：`uint` / `int`（补码）/ `f32` / `f64` / `enum` / `bytes`
- `scale` / `bias`：物理值换算 `value = raw * scale + bias`
- `enum`：原始值到标签的映射，如 `{"0": "OFF", "1": "ON"}`

## 故障规则（YAML）

`assets/fault_rules.yaml` 中每条规则包含 `id`、`subsystem`、`severity`
（info/warning/critical）、`message` 与条件树 `when`：

```yaml
when:
  all:
    - { field: batt_temp, op: lt, value: 0 }
    - { field: batt_heater, op: eq, value: "OFF" }
```

支持 `all` / `any` / `not` 嵌套组合，运算符 `eq` `ne` `lt` `le` `gt` `ge` `in`；
枚举字段可直接与标签字符串比较。分系统状态取该分系统命中规则的最高严重等级。

## 示例数据

`assets/frame_a.bin`（正常帧）与 `assets/frame_b.bin`（含电源/热控/姿态故障帧）
由脚本生成：

```bash
python3 scripts/make_sample_frames.py
```

## 测试

```bash
cargo test
```
