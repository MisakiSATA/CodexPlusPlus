#!/bin/bash

echo "=== CodexPlusPlus 推送到 GitHub ==="
echo ""
echo "当前状态："
git status
echo ""
echo "最近提交："
git log --oneline -3
echo ""
echo "---"
echo ""
echo "选项 1: 直接推送主分支（快速）"
echo "  git push origin main"
echo ""
echo "选项 2: 通过 PR 合并（推荐）"
echo "  1. git reset --hard origin/main"
echo "  2. 访问: https://github.com/MisakiSATA/CodexPlusPlus/pull/new/fix/session-sync-race-conditions"
echo "  3. 在 GitHub 上创建并合并 PR"
echo "  4. git pull origin main"
echo ""
echo "---"
echo ""
read -p "是否现在推送到 GitHub? (y/n) " -n 1 -r
echo ""
if [[ $REPLY =~ ^[Yy]$ ]]
then
    echo "推送中..."
    git push origin main
    echo ""
    echo "✅ 推送完成！"
else
    echo "取消推送。你可以稍后手动执行。"
fi
