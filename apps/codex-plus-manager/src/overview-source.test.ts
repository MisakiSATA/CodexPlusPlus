import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { describe, it } from "node:test";

const sourceSection = (source: string, start: string, end: string): string => {
  const startIndex = source.indexOf(start);
  const endIndex = source.indexOf(end, startIndex + start.length);
  assert.ok(startIndex >= 0, `missing section start: ${start}`);
  assert.ok(endIndex > startIndex, `missing section end: ${end}`);
  return source.slice(startIndex, endIndex);
};

describe("overview source", () => {
  it("does not render the JOJO Code relay promotion", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const styles = await readFile(new URL("./styles.css", import.meta.url), "utf8");
    const i18n = await readFile(new URL("./i18n-en.ts", import.meta.url), "utf8");
    const overview = sourceSection(app, "function OverviewScreen", "function RelayEnvironmentScreen");

    assert.doesNotMatch(overview, /JOJO Code|官方中转站|jojocode-overview/);
    assert.doesNotMatch(styles, /\.jojocode-overview/);
    assert.doesNotMatch(i18n, /Codex\+\+ 官方中转站|打开 JOJO Code/);
  });

  it("shows the user fork as the application repository", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const about = sourceSection(app, "function AboutScreen", "function SettingsScreen");

    assert.match(about, /github\.com\/MisakiSATA\/CodexPlusPlus/);
    assert.match(about, /https:\/\/github\.com\/MisakiSATA\/CodexPlusPlus/);
    assert.match(about, /https:\/\/github\.com\/MisakiSATA\/CodexPlusPlus\/issues/);
    assert.doesNotMatch(about, /BigPizzaV3\/CodexPlusPlus/);
  });
});
