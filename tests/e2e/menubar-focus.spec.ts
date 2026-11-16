import { test, expect } from "@playwright/test";
import { fixture, pushState, type FixtureControl } from "./native-fixture";
import { bootstrap } from "../fixtures";

for (const [platform, userAgent] of [
  [
    "macOS",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
  ],
  [
    "Windows",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/144.0.0.0 Safari/537.36",
  ],
] as const) {
  test.describe(`${platform} tray panel focus`, () => {
    test.use({ userAgent });

    test("opens without selecting a button and preserves deliberate keyboard focus", async ({
      page,
    }) => {
      const state = bootstrap();
      await fixture(page, { state });
      await page.setViewportSize({ width: 400, height: 760 });
      await page.goto("/#/menubar");
      const panel = page.getByRole("main");
      const settings = page.getByRole("button", { name: "设置", exact: true });
      const lastControl = () =>
        page.evaluate(
          () =>
            (window as unknown as { __ORBIT_TEST__: FixtureControl })
              .__ORBIT_TEST__.lastRequests.menubar_control,
        );
      await expect(panel).toBeFocused();
      await expect(panel).toHaveCSS("outline-style", "none");
      await expect(settings).not.toBeFocused();
      await expect(settings).toHaveCSS("outline-style", "none");
      await page.keyboard.press("Enter");
      expect(await lastControl()).not.toMatchObject({ action: "open" });

      await page.keyboard.press("Tab");
      await expect(settings).toBeFocused();
      expect(
        await settings.evaluate((element) => element.matches(":focus-visible")),
      ).toBe(true);
      await expect(settings).toHaveCSS("outline-style", "solid");
      await page.keyboard.press("Tab");
      const refresh = page.getByRole("button", {
        name: "刷新数据",
        exact: true,
      });
      await expect(refresh).toBeFocused();
      state.state_revision++;
      await pushState(page, state);
      await expect(refresh).toBeFocused();

      for (const previous of [
        settings,
        page.getByRole("button", { name: "更多操作", exact: true }),
      ]) {
        await previous.focus();
        await page.keyboard.press("Escape");
        // The preloaded native WebView can restore its last active button.
        // Send the same opened notification that follows native window focus.
        await page.evaluate(() =>
          (
            window as unknown as {
              __TAURI_INTERNALS__: {
                invoke: (command: string, args: unknown) => Promise<unknown>;
              };
            }
          ).__TAURI_INTERNALS__.invoke("menubar_control", {
            request: { action: "ready" },
          }),
        );
        await expect(panel).toBeFocused();
        await expect(panel).toHaveCSS("outline-style", "none");
        await expect(previous).not.toBeFocused();
        await expect(previous).toHaveCSS("outline-style", "none");
        await page.keyboard.press("Tab");
        await expect(settings).toBeFocused();
      }
      await page.keyboard.press("Enter");
      expect(await lastControl()).toEqual({
        action: "open",
        destination: "settings",
      });
    });
  });
}
