import { expect, test } from "@playwright/test";
import { apiKeyPage } from "../api-key-fixtures";
import { bootstrap, syntheticCodexModel } from "../fixtures";
import { calls, fixture } from "./native-fixture";

for (const width of [1280, 800, 460]) {
  test(`Claude mapping hint stays beside Connect at ${width}px`, async ({
    page,
  }, info) => {
    await fixture(page);
    await page.setViewportSize({ width, height: 1000 });
    await page.goto("/#/access");
    const client = page.locator("#client-claude");
    const hint = client.locator("#connect-reason-claude");
    const button = client.getByRole("button", {
      name: "接入 Orbit",
      exact: true,
    });
    await expect(hint).toHaveText(
      "请为 Sonnet、Opus、Haiku 分别选择实际调用的模型。",
    );
    await expect(button).toBeDisabled();
    await expect(client.locator(".claude-mapping")).toHaveCount(3);
    const hintBox = (await hint.boundingBox())!;
    const buttonBox = (await button.boundingBox())!;
    expect(hintBox.x + hintBox.width).toBeLessThanOrEqual(buttonBox.x);
    expect(
      Math.abs(
        hintBox.y + hintBox.height / 2 - buttonBox.y - buttonBox.height / 2,
      ),
    ).toBeLessThan(2);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    await client.screenshot({
      path: info.outputPath("claude-before-connect.png"),
    });
  });

  test(`Claude mapping stays visible and shows the confirmed binding at ${width}px`, async ({
    page,
  }, info) => {
    const state = bootstrap();
    state.clients = state.clients.filter(
      (client) => client.client === "claude",
    );
    state.switch_snapshots = [
      {
        id: "owned-claude",
        client: "claude",
        account_id: state.session.account!.id,
        account_label: state.session.account!.label,
        panel_origin: state.session.panel_origin!,
        installation_id: "synthetic-installation",
        created_at: "2026-11-17T00:00:00Z",
        expires_at: "2099-01-01T00:00:00Z",
        file_count: 3,
        state: "available",
      },
    ];
    const keys = apiKeyPage();
    keys.selected = keys.items[1]!;
    keys.bindings = keys.bindings.map((binding) => ({
      ...binding,
      current: false,
      claude_mapping:
        binding.client === "claude"
          ? {
              sonnet: "bound-sonnet",
              opus: "retired-opus",
              haiku: "bound-haiku",
            }
          : null,
    }));
    const sonnet = syntheticCodexModel("bound-sonnet");
    sonnet.label = "已绑定的 Sonnet 模型";
    const haiku = syntheticCodexModel("bound-haiku");
    haiku.label = "已绑定的 Haiku 模型";
    await fixture(page, { state, models: [sonnet, haiku], apiKeys: keys });
    await page.setViewportSize({ width, height: 1000 });
    await page.goto("/#/access");
    const client = page.locator("#client-claude");
    await expect(client.locator(".claude-model-config")).toBeVisible();
    await expect(client.locator(".claude-mapping")).toHaveCount(3);
    for (const [role, label] of [
      ["sonnet", sonnet.label],
      ["opus", "retired-opus"],
      ["haiku", haiku.label],
    ]) {
      await expect(client.locator(`#claude-${role}`)).toContainText(label!);
      await expect(client.locator(`#claude-${role}`)).toBeDisabled();
    }
    await expect(
      client.getByRole("button", { name: "取消接入", exact: true }),
    ).toBeVisible();
    await expect(
      client.getByRole("button", { name: "接入 Orbit", exact: true }),
    ).toHaveCount(0);
    expect(await calls(page, "apply_client_config")).toBe(0);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    await client.screenshot({
      path: info.outputPath("claude-after-connect.png"),
    });
  });
}

test("Claude mapping does not display a different account's binding", async ({
  page,
}) => {
  const state = bootstrap();
  state.switch_snapshots = [
    {
      id: "foreign-claude",
      client: "claude",
      account_id: "another-account",
      account_label: "Another account",
      panel_origin: state.session.panel_origin!,
      installation_id: "another-installation",
      created_at: "2026-11-17T00:00:00Z",
      expires_at: "2099-01-01T00:00:00Z",
      file_count: 3,
      state: "available",
    },
  ];
  const keys = apiKeyPage();
  keys.bindings = keys.bindings.map((binding) => ({
    ...binding,
    claude_mapping:
      binding.client === "claude"
        ? {
            sonnet: "private-other-model",
            opus: "private-other-model",
            haiku: "private-other-model",
          }
        : null,
  }));
  await fixture(page, { state, apiKeys: keys });
  await page.goto("/#/access");
  for (const role of ["sonnet", "opus", "haiku"]) {
    await expect(page.locator(`#claude-${role}`)).toBeDisabled();
    await expect(page.locator(`#claude-${role}`)).toContainText(
      "选择获准的模型",
    );
  }
  await expect(page.locator("#client-claude")).not.toContainText(
    "private-other-model",
  );
});
