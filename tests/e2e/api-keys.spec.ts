import { expect, test } from "@playwright/test";
import { apiKeyPage } from "../api-key-fixtures";
import { bootstrap } from "../fixtures";
import { calls, fixture, pushState } from "./native-fixture";

test("API Key list aligns its actions and creates and selects a key while preserving client bindings", async ({
  page,
}) => {
  await fixture(page);
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/#/api-keys");
  await expect(
    page.getByRole("heading", { name: "API Key", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".key-table tbody tr")).toHaveCount(3);
  await expect(
    page.getByText(
      "默认密钥用于新建图片任务与后续接入；已接入的工具分别保留当前绑定。",
    ),
  ).toHaveCount(0);
  const group = page.locator(".key-action-group").first();
  const details = await group
    .getByRole("button", { name: "详情", exact: true })
    .boundingBox();
  const more = await group
    .getByRole("button", { name: "日常开发的更多操作", exact: true })
    .boundingBox();
  expect(
    Math.abs(details!.y + details!.height / 2 - more!.y - more!.height / 2),
  ).toBeLessThan(1);
  await page.getByRole("button", { name: "创建密钥", exact: true }).click();
  await page.getByLabel("密钥名称", { exact: true }).fill("本地测试密钥");
  await page.locator("#key-mode").click();
  await page.getByRole("option", { name: "累计额度", exact: true }).click();
  await page
    .getByLabel("限额金额", { exact: false })
    .fill("9007199254740993.12345678");
  await page.getByRole("button", { name: "创建并使用", exact: true }).click();
  await expect(page.locator(".key-current-title")).toContainText(
    "本地测试密钥",
  );
  await expect(page.locator(".key-bound-name").first()).toHaveText("日常开发");
  await expect(
    page.getByRole("button", { name: "更新接入", exact: true }),
  ).toHaveCount(2);
  expect(await calls(page, "mutate_api_key")).toBe(1);
  expect(await calls(page, "select_api_key")).toBe(1);
  expect(
    await page.evaluate(
      () =>
        (
          window as unknown as {
            __ORBIT_TEST__: {
              lastRequests: Record<
                string,
                { changes: { spending_limit: unknown } }
              >;
            };
          }
        ).__ORBIT_TEST__.lastRequests.mutate_api_key?.changes.spending_limit,
    ),
  ).toBe("9007199254740993.12345678");
  await page.screenshot({
    path: ".cache/api-keys-demo/api-keys-light.png",
    fullPage: true,
  });
});

test("Panel-cleaned list has no legacy keys, expired keys cannot be selected and reveal stays temporary", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/api-keys");
  await page.getByRole("button", { name: "切换密钥", exact: true }).click();
  await expect(page.getByRole("radio").nth(2)).toBeDisabled();
  await expect(page.getByRole("radio")).toHaveCount(3);
  await expect(page.getByText("旧版自动密钥", { exact: true })).toHaveCount(0);
  await page.getByRole("radio").nth(1).check();
  await page.getByRole("button", { name: "使用此密钥", exact: true }).click();
  await expect(page.locator(".key-current-title")).toContainText("图片创作");
  await page.getByRole("button", { name: "详情", exact: true }).first().click();
  await page.getByRole("button", { name: "查看与复制", exact: true }).click();
  await page.getByRole("button", { name: "显示密钥", exact: true }).click();
  await expect(page.locator(".key-reveal-value")).toHaveText(
    "orv_sk_SYNTHETIC_BROWSER_SECRET",
  );
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(page.locator(".key-reveal-value")).not.toContainText(
    "SYNTHETIC_BROWSER_SECRET",
  );
  await page.getByRole("button", { name: "复制密钥", exact: true }).click();
  expect(await calls(page, "copy_api_key")).toBe(1);
  await expect(page.locator("body")).not.toContainText(
    "SYNTHETIC_BROWSER_SECRET",
  );
});

test("readonly accounts cannot mutate or choose; usage detail carries the actor-owned key filter", async ({
  page,
}) => {
  await fixture(page, { apiKeys: apiKeyPage({ can_write: false }) });
  await page.goto("/#/api-keys");
  await expect(
    page.getByRole("button", { name: "创建密钥", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "切换密钥", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "详情", exact: true }).first().click();
  await page.getByRole("button", { name: "查看用量记录", exact: true }).click();
  await expect(page).toHaveURL(/api_key_id=A{26}/);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as unknown as {
              __ORBIT_TEST__: {
                lastRequests: Record<string, Record<string, unknown>>;
              };
            }
          ).__ORBIT_TEST__.lastRequests.read_usage_summary?.api_key_id,
      ),
    )
    .toBe("A".repeat(26));
  expect(await calls(page, "mutate_api_key")).toBe(0);
});

test("API Key layout stays inside the window at smaller widths and with dark theme", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/api-keys");
  await expect(page.locator(".key-table tbody tr")).toHaveCount(3);
  const dark = bootstrap("synthetic-account-a", 2);
  dark.settings.theme = "dark";
  await pushState(page, dark);
  for (const width of [1280, 800, 460]) {
    await page.setViewportSize({ width, height: 900 });
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(width);
    await page.screenshot({
      path: ".cache/api-keys-demo/api-keys-dark-" + width + ".png",
      fullPage: true,
    });
  }
});

test("access uses the API Key page default, shows a compact confirmation and automatically dismisses its notice", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/api-keys");
  await page.getByRole("button", { name: "切换密钥", exact: true }).click();
  await page.getByRole("radio").nth(1).check();
  await page.getByRole("button", { name: "使用此密钥", exact: true }).click();
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "接入", exact: true })
    .click();
  await expect(page.locator("#access-api-key")).toHaveCount(0);
  await page
    .locator("#client-codex")
    .getByRole("button", { name: "接入 Orbit", exact: true })
    .click();
  const dialog = page.getByRole("dialog", { name: "确认接入", exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(".preview-context")).toContainText("图片创作");
  await expect(dialog.locator(".preview-context > div")).toHaveCount(4);
  await expect(
    dialog.locator("h3, h4, pre, .path-list, .impact-list"),
  ).toHaveCount(0);
  await expect(
    dialog.getByRole("button", { name: "确认接入", exact: true }),
  ).toBeEnabled();
  await dialog.getByRole("button", { name: "确认接入", exact: true }).click();
  await expect(page.locator(".toast")).toHaveText(
    "接入配置已保存，请重启应用。",
  );
  await expect(page.locator(".toast")).toHaveCount(0, { timeout: 6000 });
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "API Key", exact: true })
    .click();
  await expect(page.locator(".key-current-title")).toContainText("图片创作");
  const binding = page.locator(".key-binding").filter({ hasText: "Codex" });
  await expect(binding.locator(".key-bound-name")).toHaveText("图片创作");
  await expect(binding).toContainText("已绑定");
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "接入", exact: true })
    .click();
  const client = page.locator("#client-codex");
  await expect(
    client.getByRole("button", { name: "取消接入", exact: true }),
  ).toHaveCount(1);
  await client.getByRole("button", { name: "取消接入", exact: true }).click();
  const cancel = page.getByRole("dialog", {
    name: "确认取消接入",
    exact: true,
  });
  await expect(cancel.locator(".preview-context > div")).toHaveCount(2);
  await expect(cancel.locator("h3, pre, .path-list, .impact-list")).toHaveCount(
    0,
  );
  await cancel
    .getByRole("button", { name: "确认取消接入", exact: true })
    .click();
  await expect(cancel).toHaveCount(0);
  await expect(
    client.getByRole("button", { name: "接入 Orbit", exact: true }),
  ).toBeVisible();
});

for (const client of ["codex", "claude"] as const) {
  test(
    client +
      " key update reviews restoration before applying the new credential",
    async ({ page }) => {
      const state = bootstrap();
      const keys = apiKeyPage();
      const selected = keys.items[1]!;
      keys.selected = selected;
      keys.bindings = keys.bindings.map((binding) => ({
        ...binding,
        current: false,
        claude_mapping:
          binding.client === "claude"
            ? {
                sonnet: "synthetic-text",
                opus: "synthetic-text",
                haiku: "synthetic-text",
              }
            : null,
      }));
      state.connections[0]!.credential_revision = selected.key_revision;
      state.switch_snapshots = [
        {
          id: "synthetic-upgrade-backup",
          client,
          account_id: state.session.account!.id,
          account_label: state.session.account!.label,
          panel_origin: state.session.panel_origin!,
          installation_id: "synthetic-installation",
          created_at: "2026-11-01T00:00:00Z",
          expires_at: "2099-01-01T00:00:00Z",
          file_count: client === "claude" ? 3 : 2,
          state: "available",
        },
      ];
      await fixture(page, { state, apiKeys: keys });
      await page.goto("/#/api-keys");
      await page
        .locator(".key-binding")
        .filter({ hasText: client === "claude" ? "Claude Desktop" : "Codex" })
        .getByRole("button", { name: "更新接入", exact: true })
        .click();
      const dialog = page.getByRole("dialog");
      const restoreLabel =
        client === "claude" ? "确认取消接入" : "确认取消接入";
      await expect(
        dialog.getByRole("button", { name: restoreLabel, exact: true }),
      ).toBeEnabled();
      expect(await calls(page, "apply_client_config")).toBe(0);
      await dialog
        .getByRole("button", { name: restoreLabel, exact: true })
        .click();
      await expect(
        dialog.getByRole("button", { name: "确认接入", exact: true }),
      ).toBeEnabled();
      await expect(dialog.locator(".preview-context").first()).toContainText(
        selected.name,
      );
      expect(await calls(page, "restore_client_config")).toBe(1);
      expect(await calls(page, "apply_client_config")).toBe(0);
      await dialog
        .getByRole("button", { name: "确认接入", exact: true })
        .click();
      await expect(dialog).toHaveCount(0);
      expect(await calls(page, "apply_client_config")).toBe(1);
    },
  );
}
