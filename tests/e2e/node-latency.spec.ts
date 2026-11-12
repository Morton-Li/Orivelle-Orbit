import { expect, test } from "@playwright/test";
import { bootstrap } from "../fixtures";
import { calls, fixture } from "./native-fixture";

test("node latency updates in the selector without disturbing a configuration preview", async ({
  page,
}) => {
  await page.clock.install();
  const state = bootstrap();
  state.nodes.push({ ...state.nodes[0]!, id: "node-b", label: "Second node" });
  state.nodes.push({
    ...state.nodes[0]!,
    id: "node-offline",
    label: "Offline node",
    available: false,
  });
  await fixture(page, { state });
  await page.goto("/#/access");
  const selector = page.locator("#access-node");
  await expect(selector).toContainText("82 ms");
  await selector.click();
  await expect(
    page.getByRole("option", { name: "Synthetic node 82 ms", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("option", { name: "Second node 132 ms", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("option", {
      name: "Offline node 节点当前不可用",
      exact: true,
    }),
  ).toHaveAttribute("data-disabled", "");
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "刷新延时", exact: true }).click();
  await expect.poll(() => calls(page, "probe_nodes")).toBe(2);
  await page
    .getByRole("button", { name: "接入 Orbit", exact: true })
    .first()
    .click();
  const confirm = page
    .getByRole("dialog")
    .getByRole("button", { name: "确认保存本地配置" });
  await expect(confirm).toBeEnabled();
  const accountReads = await calls(page, "refresh_account");
  const modelReads = await calls(page, "list_models");
  await page.clock.fastForward(30_000);
  await expect.poll(() => calls(page, "probe_nodes")).toBe(3);
  await expect(confirm).toBeEnabled();
  expect(await calls(page, "refresh_account")).toBe(accountReads);
  expect(await calls(page, "list_models")).toBe(modelReads);
  expect(await calls(page, "select_node")).toBe(0);
  expect(await calls(page, "apply_client_config")).toBe(0);
  await page.keyboard.press("Escape");
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "首页", exact: true })
    .click();
  await page.clock.fastForward(60_000);
  expect(await calls(page, "probe_nodes")).toBe(3);
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "接入", exact: true })
    .click();
  await expect(selector).toContainText("82 ms");
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: ".cache/evidence/node-latency-narrow.png",
    fullPage: true,
  });
});
