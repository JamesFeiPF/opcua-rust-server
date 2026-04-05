# OPC UA ReadWrite Server

基于 Rust async-opcua 库实现的 OPC UA 服务器，支持从 CSV 文件动态加载变量节点，提供读写权限控制和数据模拟功能。

## 功能特性

- **动态变量加载**：从 `tags.csv` 文件读取标签配置，自动创建 OPC UA 节点
- **读写权限控制**：通过 CSV 中的 `readonly` 字段控制变量是否可写
- **数据模拟**：部分变量支持自动变化模拟（设置 `changerate` 参数）
- **HTTP API**：提供 REST API 用于动态添加/删除标签
- **实时更新**：后台任务定期更新模拟变量的值

## 快速开始

### 构建项目

```bash
cargo build --release
```

### 配置 tags.csv

在可执行文件目录下创建 `tags.csv` 文件，格式如下：

```csv
name,node_id,data_type,initial_value,description,unit,readonly,changerate
Temperature_Sensor_1,ns=2;s=Temperature_Sensor_1,Double,25.5,Temperature sensor,°C,false,0
Pressure_Sensor_1,ns=2;s=Pressure_Sensor_1,Double,1013.25,Pressure reading,hPa,false,0
Ambient_Temperature,ns=2;s=Ambient_Temperature,Double,22.0,Ambient temp,°C,true,0.1
```

**字段说明**：
| 字段 | 说明 |
|------|------|
| name | 变量显示名称 |
| node_id | OPC UA 节点 ID (如 `ns=2;s=TagName`) |
| data_type | 数据类型 (目前只支持 Double) |
| initial_value | 初始值 |
| description | 变量描述 |
| unit | 单位 |
| readonly | `true`=只读, `false`=可写 |
| changerate | 变化率 (0=不变, >0=模拟变化) |

### 运行服务器

```bash
./target/release/opcua_rust.exe
```

服务器启动参数：
- OPC UA 端点: `opc.tcp://0.0.0.0:4840`
- HTTP API 端点: `http://127.0.0.1:8080`

## 使用 UAExpert 测试

1. 下载并安装 [UAExpert](https://www.unified-automation.com/products/development-tools/uaexpert.html)
2. 添加服务器端点: `opc.tcp://127.0.0.1:4840`
3. 连接服务器
4. 浏览节点: `Objects > ReadWriteSimulationFolder`
5. 右键可写变量 → `Write` 写入新值

## HTTP API

### 添加标签

```bash
POST http://127.0.0.1:8080/api/addTag
Content-Type: application/json

{"tagName": "NewTag", "value": 42.5}
```

响应：
```json
{"success":true,"nodeId":"ns=2;s=Tag_X","idx":X,"value":42.5}
```

### 删除标签

```bash
POST http://127.0.0.1:8080/api/deleteTag
Content-Type: application/json

{"idx": 0}
```

响应：
```json
{"success":true}
```

## 项目结构

```
opcua_rust_readwrite_final/
├── src/bin/server.rs     # 主服务器代码
├── Cargo.toml            # 项目配置
├── server.conf          # OPC UA 服务器配置
├── tags.csv             # 标签配置文件
└── target/release/
    └── opcua_rust.exe   # 编译后的可执行文件
```

## 依赖

- async-opcua = "0.18" (server feature)
- async-opcua-nodes = "0.18"
- rand = "0.8"
- tokio = "1.0"
- csv = "1.3"
- serde = "1.0"
- parking_lot = "0.12"
- tiny_http = "0.12"

## 许可证

MPL-2.0
