import { expect, test } from "@playwright/test";
import { bootstrap } from "../fixtures";
import { fixture, calls, pushState } from "./native-fixture";
test("usage shows account-wide statistics, filters and cursors without inference access", async ({
  page,
}) => {
  const state = bootstrap();
  state.connections[0]!.node_id = null;
  state.connections[0]!.permission_state = "readonly";
  state.session.inference_credential_status = "permission_denied";
  await fixture(page, { state });
  await page.setViewportSize({ width: 1280, height: 940 });
  await page.goto("/#/usage");
  await expect(
    page.getByRole("heading", { name: "用量", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".usage-stat")).toHaveCount(4);
  await expect(page.locator(".usage-table tbody tr")).toHaveCount(4);
  await expect(page.locator(".usage-stat-accent")).toContainText("USD");
  await page.screenshot({
    path: ".cache/usage-demo/usage-light.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "已确认消费", exact: true }).click();
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "上一页", exact: true }),
  ).toBeEnabled();
  await page.locator("#usage-outcome").click();
  await page.getByRole("option", { name: "失败", exact: true }).click();
  await expect(page.locator(".usage-table tbody tr")).toHaveCount(1);
  await page.getByRole("button", { name: "查看详情", exact: true }).click();
  await expect(page.locator(".usage-detail-grid")).toContainText("a1000000-");
  const dark = bootstrap("synthetic-account-a", 2);
  dark.settings.theme = "dark";
  await pushState(page, dark);
  await page.screenshot({
    path: ".cache/usage-demo/usage-dark.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 800, height: 800 });
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(800);
  expect(await calls(page, "create_image_task")).toBe(0);
});

test("usage errors leave space above statistics and align refresh on the right", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/usage");
  await expect(
    page.locator(".usage-stat").first().locator("strong"),
  ).not.toHaveText("—");
  const previous = (await page.locator(".usage-stat-grid").textContent()) ?? "";
  await page.evaluate(() => {
    const native = (
      window as unknown as {
        __TAURI_INTERNALS__: {
          invoke: (
            command: string,
            args?: Record<string, unknown>,
          ) => Promise<unknown>;
        };
      }
    ).__TAURI_INTERNALS__;
    const invoke = native.invoke;
    const failing = new Set(["read_usage_summary", "list_usage_records"]);
    native.invoke = async (command, args) => {
      if (failing.delete(command))
        throw { code: "synthetic_usage_unavailable" };
      return invoke(command, args);
    };
  });
  await page.getByRole("button", { name: "刷新用量", exact: true }).click();
  await expect(page.locator(".usage-error")).toHaveCount(2);
  await expect(page.locator(".usage-stat-grid")).toHaveText(previous);
  const summaryError = page.locator(".usage-error").first();
  for (const width of [1280, 800, 460]) {
    await page.setViewportSize({ width, height: 940 });
    for (const alert of await page.locator(".usage-error").all()) {
      const box = await alert.boundingBox();
      const message = await alert.locator("span").boundingBox();
      const button = await alert
        .getByRole("button", { name: "刷新数据", exact: true })
        .boundingBox();
      expect(button!.x + button!.width).toBeGreaterThan(
        box!.x + box!.width - 20,
      );
      expect(button!.x).toBeGreaterThanOrEqual(
        message!.x + message!.width + 12,
      );
    }
    const notice = await summaryError.boundingBox();
    const statistics = await page.locator(".usage-stat-grid").boundingBox();
    expect(statistics!.y - notice!.y - notice!.height).toBeGreaterThanOrEqual(
      17,
    );
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(width);
    if (width !== 800)
      await page.screenshot({
        path: `.cache/usage-demo/error-layout-${width}.png`,
        fullPage: true,
      });
  }
  await summaryError
    .getByRole("button", { name: "刷新数据", exact: true })
    .click();
  await expect(page.locator(".usage-error")).toHaveCount(0);
  await expect(page.locator(".usage-stat-grid")).toHaveText(previous);
});
