# OPC UA Rust 服务器开发笔记

## 从零开始：我的 OPC UA 服务器开发完整记录

> 这是一篇详细记录我使用 Rust 开发 OPC UA 服务器全过程的笔记，包含从环境搭建到最终解决问题的完整经历。适合想要学习 OPC UA 和 Rust 网络编程的初学者参考。

---

## 背景介绍

OPC UA (Open Platform Communications Unified Architecture) 是一种工业自动化通信协议，广泛用于工厂设备之间的数据交换。之前我需要创建一个 OPC UA 服务器，用于模拟工业传感器的数据，并且要支持客户端（如 UAExpert）能够写入数值。

技术栈选择：
- **后端**: Rust + async-opcua 库
- **客户端**: HTML + JavaScript (浏览器端轮询)

---

## 第一阶段：环境搭建与基础服务器

### 1.1 安装 Rust 环境

首先需要安装 Rust 编程语言：

```bash
# 安装 Rust (Windows)
# 下载 rustup-init.exe 并运行

# 验证安装
rustc --version
cargo --version
```

### 1.2 创建新项目

```bash
cargo new opcua_rust_readwrite_final
cd opcua_rust_readwrite_final
```

### 1.3 添加依赖

编辑 `Cargo.toml`：

```toml
[dependencies]
async-opcua = { version = "0.18", features = ["server"] }
rand = { version = "0.8", features = ["small_rng"] }
tokio = { version = "1.0", features = ["full"] }
csv = "1.3"
serde = { version = "1.0", features = ["derive"] }
parking_lot = "0.12"
tiny_http = "0.12"
```

### 1.4 第一个可运行的服务器

最初我写了一个简单的服务器，只能读取 CSV 文件中的变量并展示：

```rust
use opcua::server::address_space::Variable;
use opcua::server::node_manager::memory::{simple_node_manager, InMemoryNodeManager, SimpleNodeManagerImpl};
use opcua::server::ServerBuilder;
use opcua::types::{BuildInfo, DateTime, NodeId};

#[tokio::main]
async fn main() {
    // 构建服务器
    let (server, handle) = ServerBuilder::new()
        .with_config_from("server.conf")
        .build_info(BuildInfo {
            product_uri: "urn:RustOPCUA:ReadWriteServer".into(),
            manufacturer_name: "Rust OPC UA".into(),
            product_name: "Rust OPC UA ReadWrite Server".into(),
            software_version: "0.1.0".into(),
            build_number: "1".into(),
            build_date: DateTime::now(),
        })
        .with_node_manager(simple_node_manager(...))
        .build()
        .unwrap();
    
    // ... 添加变量的代码
    server.run().await.unwrap();
}
```

**💡 心得**：OPC UA 服务器开发比想象的要简单，async-opcua 库封装了很多底层细节。我只需要关注业务逻辑（如何定义变量、如何更新值）。

---

## 第二阶段：CSV 配置与变量加载

### 2.1 设计 CSV 格式

为了让服务器灵活配置，我设计了一个 CSV 文件格式：

```csv
name,node_id,data_type,initial_value,description,unit,readonly,changerate
Temperature_Sensor_1,ns=2;s=Temperature_Sensor_1,Double,25.5,Temperature sensor,°C,false,0
Pressure_Sensor_1,ns=2;s=Pressure_Sensor_1,Double,1013.25,Pressure reading,hPa,false,0
Ambient_Temperature,ns=2;s=Ambient_Temperature,Double,22.0,Ambient temp,°C,true,0.1
```

**字段说明**：
- `name`: 变量显示名称
- `node_id`: OPC UA 节点 ID，格式为 `ns=2;s=xxx`
- `data_type`: 数据类型（目前支持 Double）
- `initial_value`: 初始值
- `description`: 描述
- `unit`: 单位
- `readonly`: 是否只读（true/false）
- `changerate`: 变化率（用于模拟数据波动）

### 2.2 解析 CSV

```rust
fn read_csv_tags() -> Vec<TagInfo> {
    let csv_path = std::env::current_exe()
        .unwrap()
        .parent().unwrap()
        .join("tags.csv");
    
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(&csv_path)
        .expect("Failed to open CSV");
    
    for result in reader.deserialize() {
        let config: TagConfig = result.expect("Failed to read CSV record");
        // 解析 node_id 字符串
        let node_id = if config.node_id.starts_with("ns=") {
            let parts: Vec<&str> = config.node_id.split(";s=").collect();
            NodeId::new(parts[0][3..].parse().unwrap(), parts[1])
        } else {
            NodeId::new(2, config.node_id.clone())
        };
        
        tags.push(TagInfo {
            node_id,
            current_value: config.initial_value,
            display_name: config.name,
            // ...
        });
    }
    tags
}
```

**💡 心得**：CSV 解析很简单，但要注意路径处理。服务器通常在特定目录运行，所以用 `current_exe()` 来定位配置文件更可靠。

---

## 第三阶段：数据模拟功能

### 3.1 后台任务更新数据

为了让"传感器"数据看起来像真的，我添加了一个后台任务定期更新数值：

```rust
tokio::spawn(async move {
    let mut rng = SmallRng::from_entropy();
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    
    loop {
        interval.tick().await;
        let tags = tags_for_update.read();
        
        let mut updates = Vec::new();
        for tag in tags.iter() {
            if tag.changerate > 0.0 {
                // 根据变化率随机调整数值
                let change_pct = (rng.gen::<f64>() - 0.5) * tag.changerate * 0.01;
                let new_value = tag.current_value * (1.0 + change_pct);
                updates.push((&tag.node_id, None, DataValue::new_now(new_value)));
            }
        }
        
        if !updates.is_empty() {
            manager_for_update.set_values(&subs_for_update, updates.into_iter()).ok();
        }
    }
});
```

**💡 心得**：这里有个重要的学习点：OPC UA 的值更新需要用 `set_values` 方法，而且是通过 `subscriptions` 实现的。一开始我以为直接修改变量就可以，后来才明白需要通过订阅机制来触发更新。

---

## 第四阶段：添加写功能（踩坑记录）

### 4.1 最初的尝试 - 失败

当我信心满满地尝试让变量可写时，发现即使设置了某些参数，UAExpert 客户端写入时仍然报错：

```
BadUserAccessDenied
```

### 4.2 第一次尝试：使用常量（错误）

我查阅了 async-opcua 文档，发现有 `set_writable()` 方法，试了一下没用。然后我尝试自己定义常量：

```rust
// 错误的做法！
const ACCESS_READ_ONLY: u8 = 0x01;
const ACCESS_READ_WRITE: u8 = 0x03;

variable.set_access_level(ACCESS_READ_WRITE);  // ❌ 编译报错
```

错误信息：
```
error[E0308]: mismatched types
   expected `AccessLevel`, found `u8`
```

原来 `set_access_level()` 方法需要的是 `AccessLevel` 类型，不是 `u8`！

### 4.3 解决方案：使用 bitflags

经过大量搜索，我终于找到了正确的方法：

```rust
use opcua_nodes::AccessLevel;  // 从 opcua-nodes crate 导入

fn make_access_level(readable: bool, writable: bool) -> AccessLevel {
    let mut bits = AccessLevel::empty();
    if readable { bits.insert(AccessLevel::CURRENT_READ); }
    if writable { bits.insert(AccessLevel::CURRENT_WRITE); }
    bits
}

// 使用
let access_level = make_access_level(true, !tag.readonly);
variable.set_access_level(access_level);
variable.set_user_access_level(access_level);
```

### 4.4 添加依赖

需要在 `Cargo.toml` 中添加：

```toml
async-opcua-nodes = "0.18"
```

**🔧 问题根源分析**：

OPC UA 的 AccessLevel 是一个 bitflags 类型，而不是简单的整数。库内部定义了：
- `CURRENT_READ` (0x01) - 可读
- `CURRENT_WRITE` (0x02) - 可写

这些常量定义在 `opcua_nodes` crate 中，需要显式导入。

**💡 心得**：这是本次开发最大的坑！Rust 的类型系统很严格，不会自动转换类型。文档和搜索能力非常重要。

---

## 第五阶段：HTTP API 与动态管理

### 5.1 添加标签的 API

为了让系统更灵活，我添加了一个简单的 HTTP API：

```rust
std::thread::spawn(move || {
    let server = tiny_http::Server::http("127.0.0.1:8080").unwrap();
    
    for mut request in server.incoming_requests() {
        if request.url() == "/api/addTag" {
            // 解析 JSON
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).ok();
            
            let tag_name = parse_json_string(&body, "tagName").unwrap();
            let init_value = parse_json_number(&body, "value").unwrap();
            
            // 添加新变量到地址空间
            let mut var = Variable::new(&node_id, &tag_name, &description, init_value);
            let access_level = make_access_level(true, true);
            var.set_access_level(access_level);
            var.set_user_access_level(access_level);
            
            address_space.add_variables(vec![var], &folder_id);
            
            // 返回响应
            let resp = format!(r#"{{"success":true,"nodeId":"{}"}}"#, node_id);
            request.respond(...);
        }
    }
});
```

**💡 心得**：使用 `tiny_http` 库实现简单的 HTTP 服务很方便。解析 JSON 部分是手写的简单解析器，对于简单的场景够用了。

---

## 第六阶段：HTML 客户端开发

### 6.1 轮询策略

浏览器不能直接使用 OPC UA 协议（需要 WebSocket 转换），所以用轮询方式：

```javascript
async function pollTags() {
    try {
        const response = await fetch('/api/tags');
        const data = await response.json();
        
        // 每次最多获取 500 个节点，分页处理
        const page = Math.floor(currentPage / 500);
        const start = page * 500;
        const end = start + 500;
        
        renderTable(data.slice(start, end));
    } catch (e) {
        console.error('Poll error:', e);
    }
}

// 每 500ms 轮询一次
setInterval(pollTags, 500);
```

### 6.2 Canvas 渲染表格

为了性能，使用 Canvas 渲染大量数据：

```javascript
const canvas = document.getElementById('tableCanvas');
const ctx = canvas.getContext('2d');

// 虚拟滚动：只渲染可见区域
function renderVisibleRows() {
    const startRow = Math.floor(scrollTop / rowHeight);
    const endRow = Math.min(startRow + visibleRows, data.length);
    
    for (let i = startRow; i < endRow; i++) {
        const y = (i - startRow) * rowHeight;
        // 绘制行...
    }
}
```

**💡 心得**：OPC UA 服务器可能有很多节点（上千个），DOM 渲染会很慢。Canvas + 虚拟滚动是处理大数据量表格的有效方案。

---

## 最终成果

### 功能总结

1. ✅ 从 CSV 文件加载 OPC UA 变量节点
2. ✅ 支持读写权限控制（通过 CSV 的 readonly 字段）
3. ✅ 后台自动模拟数据变化
4. ✅ HTTP API 动态添加/删除标签
5. ✅ HTML Web 客户端展示

### 项目结构

```
opcua_rust_readwrite_final/
├── src/bin/server.rs     # 主服务器代码
├── Cargo.toml            # 项目配置
├── server.conf           # OPC UA 配置
├── tags.csv              # 标签定义
└── README.md             # 项目说明
```

### GitHub 仓库

项目已上传至：https://github.com/JamesFeiPF/opcua-rust-server

---

## 经验总结

### 技术层面

1. **OPC UA 权限模型**：要使变量可写，需要同时设置 `access_level` 和 `user_access_level`，两者都要包含 `CURRENT_WRITE` 位
2. **Rust 类型安全**：不要假设类型可以自动转换，文档和编译器错误信息很重要
3. **后台任务**：使用 `tokio::spawn` 创建后台任务处理定时任务
4. **线程安全**：使用 `Arc<RwLock<...>>` 在线程间共享数据

### 学习层面

1. **搜索能力**：遇到问题先搜索官方文档和 GitHub Issues
2. **最小化复现**：先写最小可运行代码，再逐步添加功能
3. **版本兼容性**：注意库的版本，不同版本 API 可能有变化

### 调试技巧

1. 编译错误优先：先解决编译错误再运行
2. 添加日志：使用 `println!()` 调试
3. UAExpert 工具：专业的 OPC UA 客户端可以查看节点属性和测试写入

---

## 后续改进方向

1. 添加用户认证和权限管理
2. 支持更多数据类型（不只 Double）
3. 使用 WebSocket 实现真正的实时推送
4. 添加历史数据存储
5. 支持订阅（Subscription）而非轮询

---

## 参考资源

- [async-opcua 官方文档](https://docs.rs/async-opcua/latest/async_opcua/)
- [OPC UA 协议规范](https://opcfoundation.org/)
- [UAExpert 下载](https://www.unified-automation.com/products/development-tools/uaexpert.html)

---

> 感谢阅读！希望这篇笔记对你学习 OPC UA 和 Rust 开发有所帮助。如果有问题，欢迎在 GitHub 上提 Issue。
