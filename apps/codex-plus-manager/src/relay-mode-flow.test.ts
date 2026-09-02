import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const sourceSection = (source: string, start: string, end: string): string => {
  const startIndex = source.indexOf(start);
  const endIndex = source.indexOf(end, startIndex + start.length);
  assert.ok(startIndex >= 0, `missing section start: ${start}`);
  assert.ok(endIndex > startIndex, `missing section end: ${end}`);
  return source.slice(startIndex, endIndex);
};

test("official mode selects an official profile and launch mode in one switch", async () => {
  const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const section = sourceSection(app, "const switchOfficialMode", "const switchPureApiMode");

  assert.match(section, /relayMode === "official"/);
  assert.match(section, /launchMode: "relay"/);
  assert.match(section, /switchRelayProfile/);
  assert.doesNotMatch(section, /clearRelayInjection|saveLaunchMode/);
});

test("pure API mode selects a pure profile and launch mode in one switch", async () => {
  const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const section = sourceSection(app, "const switchPureApiMode", "const switchRelayProfile");

  assert.match(section, /relayMode === "pureApi"/);
  assert.match(section, /launchMode: "patch"/);
  assert.match(section, /switchRelayProfile/);
  assert.doesNotMatch(section, /applyPureApiInjection|saveLaunchMode/);
});

test("relay switch consumes the camelCase userScripts payload", async () => {
  const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const resultType = sourceSection(app, "type RelaySwitchResult", "type RelayProfileTestResult");
  const switchFlow = sourceSection(app, "const switchRelayProfile", "const copyText");

  assert.match(resultType, /userScripts:\s*unknown/);
  assert.doesNotMatch(resultType, /user_scripts/);
  assert.match(switchFlow, /user_scripts:\s*result\.userScripts/);
  assert.doesNotMatch(switchFlow, /result\.user_scripts/);
});

test("saving the active enabled relay profile reuses the transactional switch flow", async () => {
  const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const saveFlow = sourceSection(app, "  const saveDraft = async () => {", "  const switchDraft = () => {");

  assert.match(saveFlow, /isActive\s*&&\s*form\.relayProfilesEnabled/);
  assert.match(saveFlow, /await actions\.switchRelayProfile\(next,\s*form\.activeRelayId\)/);
  assert.match(saveFlow, /:\s*await onFormChange\(next\)/);
  assert.doesNotMatch(saveFlow, /saveRelayFile/);
});

test("relay profile detail closes only after its save or apply succeeds", async () => {
  const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const settingsSave = sourceSection(app, "  const saveSettingsValue", "  const resetSettings");
  const relayScreen = sourceSection(app, "function RelayScreen", "function EnvConflictNotice");
  const saveFlow = sourceSection(app, "  const saveDraft = async () => {", "  const switchDraft = () => {");

  assert.match(settingsSave, /if \(!result\) return false/);
  assert.match(settingsSave, /return isSuccessStatus\(result\.status\)/);
  assert.match(relayScreen, /return actions\.saveSettingsValue\(next,\s*true\)/);
  assert.match(saveFlow, /if \(!saved\) return;\s*onSaved\?\.\(\)/);
});

test("active relay profile cannot be deleted before switching away", async () => {
  const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const english = await readFile(new URL("./i18n-en.ts", import.meta.url), "utf8");
  const deleteButton = sourceSection(app, 'title={t("复制")}', "<Trash2");

  assert.match(deleteButton, /disabled=\{form\.relayProfiles\.length <= 1 \|\| active\}/);
  assert.match(deleteButton, /if \(active\) return;/);
  assert.match(
    deleteButton,
    /active\s*\?\s*t\("当前供应商使用中，请先切换到其他供应商再删除"\)\s*:\s*t\("删除供应商"\)/,
  );
  assert.match(english, /Switch to another provider before deleting/);
});

test("provider master switch accurately describes startup replay", async () => {
  const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const english = await readFile(new URL("./i18n-en.ts", import.meta.url), "utf8");
  const copy =
    "关闭后，手动切换和启动 Codex 都不会写入 config.toml / auth.json；开启后，通过 Codex++ 启动时会重新应用当前 API 或聚合供应商，纯官方登录配置除外。";

  assert.match(app, new RegExp(copy.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.doesNotMatch(app, /启动 Codex 时始终不会自动改这些文件/);
  assert.match(english, /launching through Codex\+\+ reapplies the current API or aggregate provider/);
});
