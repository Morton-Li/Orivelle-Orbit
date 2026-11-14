import { test, expect } from "@playwright/test";
import { fixture } from "./native-fixture";
import { bootstrap, task } from "../fixtures";

test("menu bar themes follow stored settings, keep the approved layout, and open the selected page", async ({
  page,
}, testInfo) => {
  const state = bootstrap();
  state.settings.theme = "light";
  state.tasks = [task("waiting")];
  await fixture(page, { state });
  await page.setViewportSize({ width: 400, height: 760 });
  await page.goto("/#/menubar");
  await expect(page.getByRole("main")).toBeVisible();
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  await expect(page.locator("html")).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(page.getByText("正在生成图片")).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("menubar-light.png"),
    fullPage: true,
    omitBackground: true,
  });
  const dark = structuredClone(state);
  dark.state_revision = 10;
  dark.settings.theme = "dark";
  await page.evaluate(
    (next) =>
      (
        window as unknown as {
          __ORBIT_TEST__: { pushState: (value: typeof next) => void };
        }
      ).__ORBIT_TEST__.pushState(next),
    dark,
  );
  await expect(page.locator("html")).toHaveClass(/dark/);
  await expect(page.locator("html")).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await page.screenshot({
    path: testInfo.outputPath("menubar-dark.png"),
    fullPage: true,
    omitBackground: true,
  });
  const system = structuredClone(dark);
  system.state_revision++;
  system.settings.theme = "system";
  await page.emulateMedia({ colorScheme: "light" });
  await page.evaluate(
    (next) =>
      (
        window as unknown as {
          __ORBIT_TEST__: { pushState: (value: typeof next) => void };
        }
      ).__ORBIT_TEST__.pushState(next),
    system,
  );
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.getByRole("button", { name: "管理接入" }).click();
  expect(
    await page.evaluate(
      () =>
        (
          window as unknown as {
            __ORBIT_TEST__: { lastRequests: Record<string, unknown> };
          }
        ).__ORBIT_TEST__.lastRequests.menubar_control,
    ),
  ).toEqual({ action: "open", destination: "access" });
  await page.setViewportSize({ width: 320, height: 900 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});
