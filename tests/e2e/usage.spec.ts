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
