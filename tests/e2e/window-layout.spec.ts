import { expect, test } from "@playwright/test";
import assert from "node:assert/strict";
import { bootstrap } from "../fixtures";
import { fixture } from "./native-fixture";

for (const platform of ["windows", "macos"] as const) {
  test.describe(`${platform} content scrolling`, () => {
    test.use({
      userAgent:
        platform === "windows"
          ? "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/150.0.0.0 Safari/537.36"
          : "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_0) AppleWebKit/537.36 Chrome/150.0.0.0 Safari/537.36",
    });
    for (const width of [1160, 640]) {
      test(`${width}px keeps scrolling below fixed chrome and restores navigation`, async ({
        page,
      }, testInfo) => {
        const state = bootstrap();
        state.platform = platform;
        await fixture(page, { state });
        await page.setViewportSize({ width, height: 720 });
        await page.goto("/#/settings");
        const content = page.locator(".main-shell");
        const header = page.locator(".topbar");
        const sidebar = page.locator(".sidebar");
        await expect(header).toHaveClass(new RegExp(`topbar-${platform}`));
        await expect(page.locator("#setting-theme")).toBeVisible();
        const headerBefore = await header.boundingBox();
        const sidebarBefore = await sidebar.boundingBox();
        const bounds = await content.boundingBox();
        assert(headerBefore && sidebarBefore && bounds);
        expect(bounds.y).toBeGreaterThanOrEqual(
          headerBefore.y + headerBefore.height,
        );
        expect(bounds.y + bounds.height).toBe(720);
        if (platform === "windows") {
          await expect(page.locator(".window-controls button")).toHaveCount(3);
          const close = await page.locator(".window-close").boundingBox();
          assert(close);
          expect(close.y + close.height).toBeLessThanOrEqual(bounds.y);
        }
        await page.mouse.move(bounds.x + bounds.width - 50, bounds.y + 100);
        await page.mouse.wheel(0, 650);
        const expectedScroll = Math.min(
          650,
          await content.evaluate((el) => el.scrollHeight - el.clientHeight),
        );
        expect(expectedScroll).toBeGreaterThan(0);
        await expect
          .poll(() => content.evaluate((el) => el.scrollTop))
          .toBe(expectedScroll);
        expect(await header.boundingBox()).toEqual(headerBefore);
        expect(await sidebar.boundingBox()).toEqual(sidebarBefore);
        expect(
          await page.evaluate(() => {
            const root = document.scrollingElement;
            if (!root) throw new Error("The document root is missing.");
            return {
              window: window.scrollY,
              root: root.scrollTop,
              rootHeight: root.scrollHeight,
              viewport: window.innerHeight,
            };
          }),
        ).toEqual({ window: 0, root: 0, rootHeight: 720, viewport: 720 });
        const position = await content.evaluate((el) => el.scrollTop);
        await page.screenshot({
          path: testInfo.outputPath("content-scroll.png"),
        });
        const navigation = page.getByRole("navigation");
        await navigation
          .getByRole("link", { name: "首页", exact: true })
          .click();
        await expect.poll(() => content.evaluate((el) => el.scrollTop)).toBe(0);
        await page.goBack();
        await expect(page.locator("#setting-theme")).toBeVisible();
        await expect
          .poll(() => content.evaluate((el) => el.scrollTop))
          .toBe(position);
        await navigation
          .getByRole("link", { name: "首页", exact: true })
          .click();
        await page
          .locator(".client-row")
          .filter({ hasText: "Claude" })
          .getByRole("link")
          .click();
        await expect(page.locator("#client-claude")).toBeInViewport();
        const anchor = await page.locator("#client-claude").boundingBox();
        assert(anchor);
        expect(anchor.y).toBeGreaterThanOrEqual(bounds.y);
        expect(await page.evaluate(() => window.scrollY)).toBe(0);
      });
    }
  });
}
