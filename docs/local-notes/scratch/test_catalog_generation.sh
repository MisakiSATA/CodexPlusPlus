#!/bin/bash
# 测试模型目录生成

set -e

echo "=== 测试模型目录生成 ==="
echo ""

# 1. 编译最新代码
echo "1. 编译最新代码..."
cd /data/Projects/Code/Project/CodexPlusPlus
cargo build --release --quiet
echo "   ✓ 编译完成"
echo ""

# 2. 检查二进制文件中是否包含通用模板
echo "2. 检查二进制文件是否包含通用模板..."
if strings target/release/codex-plus-plus | grep -q "generic-custom-model"; then
    echo "   ✓ 通用模板已嵌入二进制文件"
else
    echo "   ✗ 通用模板未找到！"
    exit 1
fi
echo ""

# 3. 创建临时测试目录
echo "3. 创建测试环境..."
TEST_DIR=$(mktemp -d)
echo "   测试目录: $TEST_DIR"
echo ""

# 4. 运行 relay_config 测试看能否通过
echo "4. 运行单元测试..."
cargo test -p codex-plus-core --test relay_config --quiet
echo "   ✓ 所有测试通过"
echo ""

# 5. 手动测试模型目录生成
echo "5. 手动生成测试模型目录..."
cat > "$TEST_DIR/test_catalog.json" << 'RUST_EOF'
use codex_plus_core::model_suffix::{ModelCatalogEntry, build_model_catalog_json};

fn main() {
    let entries = vec![
        ModelCatalogEntry {
            slug: "claude-opus-4".to_string(),
            display_name: "Claude Opus 4".to_string(),
            suffix_window: Some(200000),
        }
    ];

    let catalog = build_model_catalog_json(&entries, None);
    println!("{}", catalog);
}
RUST_EOF

echo "   测试代码已准备"
echo ""

# 清理
rm -rf "$TEST_DIR"

echo "=== 测试完成 ==="
