import { expect, test } from "@playwright/test";
import { bootstrap, task } from "../fixtures";
import {
  calls,
  fixture,
  pushState,
  type FixtureControl,
} from "./native-fixture";

test("unconnected UI, navigation, keyboard, modal focus and narrow container", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(
    "让 AI 工具接入 Orivelle",
  );
  await expect(page.getByText("待检测", { exact: true })).toHaveCount(2);
  await page.screenshot({
    path: ".cache/evidence/home-unconnected.png",
    fullPage: true,
  });
  const login = page
    .getByRole("button", { name: "连接账户", exact: true })
    .last();
  await login.click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(login).toBeFocused();
  await page.keyboard.press("Control+3");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(
    "选择工具，确认每次切换",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  for (const name of ["首页", "图片工作台", "接入", "设置"]) {
    await page
      .getByRole("navigation")
      .getByRole("link", { name, exact: true })
      .click();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
  }
  await page.screenshot({
    path: ".cache/evidence/settings-narrow.png",
    fullPage: true,
  });
});

test("real capability names populate client models and image edit options", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/access");
  await expect(page.locator("#model-codex option")).toHaveCount(2);
  await expect(page.locator("#model-claude option")).toHaveCount(2);
  await expect(
    page.getByText("合成测试数据 · 不代表真实接入或结算"),
  ).toBeVisible();
  await expect(
    page.getByText("待重启 / 载入未确认", { exact: true }),
  ).toHaveCount(2);
  await page.screenshot({
    path: ".cache/evidence/access-light.png",
    fullPage: true,
  });
  await page.locator("#model-codex").selectOption("synthetic-text");
  await page.getByRole("button", { name: "预览配置" }).first().click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("/fixture/config")).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "确认保存本地配置" }),
  ).toBeEnabled();
  const changed = bootstrap("synthetic-account-a", 2);
  changed.connections[0]!.revision = 2;
  await pushState(page, changed);
  await expect(
    dialog.getByRole("button", { name: "确认保存本地配置" }),
  ).toBeDisabled();
  expect(await calls(page, "apply_client_config")).toBe(0);
});

test("changing accounts clears prompt and drafts before another submit", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/images");
  await page.locator("#image-model").selectOption("synthetic-image");
  await page
    .locator("#image-prompt")
    .fill("Private synthetic account A prompt");
  await pushState(page, bootstrap("synthetic-account-b", 2));
  await expect(page.locator("#image-prompt")).toHaveValue("");
  await expect(page.locator("#image-model")).toHaveValue("");
  expect(await calls(page, "create_image_task")).toBe(0);
});

test("healthy session rotation preserves a draft while disabling submission, and expiry clears it", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/images");
  await page.locator("#image-model").selectOption("synthetic-image");
  await page.locator("#image-prompt").fill("Same actor draft during rotation");
  const rotating = bootstrap("synthetic-account-a", 2);
  rotating.session.status = "refreshing";
  await pushState(page, rotating);
  await expect(page.locator("#image-prompt")).toHaveValue(
    "Same actor draft during rotation",
  );
  await expect(page.locator("#image-model")).toHaveValue("synthetic-image");
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeDisabled();
  await pushState(page, bootstrap("synthetic-account-a", 3));
  await expect(page.locator("#image-prompt")).toHaveValue(
    "Same actor draft during rotation",
  );
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeEnabled();
  const expired = bootstrap("synthetic-account-a", 4);
  expired.session.status = "expired";
  await pushState(page, expired);
  await expect(page.locator("#image-prompt")).toHaveValue("");
  await expect(page.locator("#image-model")).toHaveValue("");
  expect(await calls(page, "create_image_task")).toBe(0);
});

test("browser authorization remains pending and supports explicit denial and cancellation", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");
  await page
    .getByRole("button", { name: "连接账户", exact: true })
    .last()
    .click();
  const dialog = page.getByRole("dialog");
  await dialog.locator("#panel-address").fill("https://panel.invalid");
  await dialog.getByRole("button", { name: "继续", exact: true }).click();
  await expect(
    dialog.getByRole("button", { name: "继续", exact: true }),
  ).toBeDisabled();
  await expect(dialog.locator("#panel-address")).toBeDisabled();
  await page.evaluate(() =>
    (
      window as unknown as { __ORBIT_TEST__: FixtureControl }
    ).__ORBIT_TEST__.pushAuthorization({
      request_id: "synthetic-login",
      state: "denied",
      error: null,
    }),
  );
  await expect(
    page.getByText("浏览器授权已拒绝，当前账户保持原状态。"),
  ).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "继续", exact: true }),
  ).toBeEnabled();
  await dialog.getByRole("button", { name: "继续", exact: true }).click();
  await expect(dialog.locator("#panel-address")).toBeDisabled();
  await dialog.getByRole("button", { name: "取消", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await calls(page, "cancel_login")).toBe(1);
  await page.evaluate(() =>
    (
      window as unknown as { __ORBIT_TEST__: FixtureControl }
    ).__ORBIT_TEST__.pushAuthorization({
      request_id: "synthetic-login",
      state: "connected",
      error: null,
    }),
  );
  await expect(page.locator(".account-summary h2")).toHaveText(
    "Synthetic synthetic-account-a",
  );
  expect(await calls(page, "create_image_task")).toBe(0);
});

test("synthetic Windows capabilities never offer an inaccessible hidden background window", async ({
  page,
}) => {
  const state = bootstrap();
  state.platform = "windows";
  await fixture(page, { state });
  await page.goto("/#/settings");
  await expect(
    page.locator('#setting-close option[value="keep_running"]'),
  ).toHaveCount(0);
  await expect(page.locator("#setting-close option")).toHaveCount(2);
});

test("update maintenance disables image submission and settings without losing a same-account draft", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/images");
  await page.locator("#image-model").selectOption("synthetic-image");
  await page
    .locator("#image-prompt")
    .fill("Draft retained through maintenance");
  const state = bootstrap("synthetic-account-a", 2);
  state.maintenance_active = true;
  await pushState(page, state);
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeDisabled();
  await expect(page.locator("#image-prompt")).toHaveValue(
    "Draft retained through maintenance",
  );
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "设置", exact: true })
    .click();
  await expect(page.locator("#setting-theme")).toBeDisabled();
  await expect(
    page.getByText("正在准备安装更新，新的图片请求与配置变更已暂停。"),
  ).toBeVisible();
  expect(await calls(page, "create_image_task")).toBe(0);
});

for (const recoverable of [false, true]) {
  test(`conflicted backup offers a gated preview while logged out; recoverable=${recoverable}`, async ({
    page,
  }) => {
    const state = bootstrap();
    state.session.status = "logged_out";
    state.session.account = null;
    state.session.inference_credential_status = "unavailable";
    state.backups = [
      {
        id: "synthetic-conflict-backup",
        client: "codex",
        account_id: "synthetic-account-a",
        account_label: "Fixture A",
        panel_origin: "https://panel.invalid",
        installation_id: "fixture-install",
        created_at: "2026-10-13T03:00:00Z",
        expires_at: "2026-11-12T03:00:00Z",
        file_count: 2,
        state: "conflict",
      },
    ];
    await fixture(page, {
      state,
      restoreMode: recoverable ? "recoverable" : "blocked",
    });
    await page.goto("/#/access");
    await page
      .locator("#backup-account")
      .selectOption("https://panel.invalid/synthetic-account-a");
    await page.getByRole("button", { name: "预览恢复", exact: true }).click();
    const apply = page
      .getByRole("dialog")
      .getByRole("button", { name: "确认应用", exact: true });
    if (recoverable) {
      await expect(apply).toBeEnabled();
      await apply.click();
      expect(await calls(page, "restore_client_config")).toBe(1);
      await expect(page.getByRole("dialog")).toHaveCount(0);
    } else {
      await expect(apply).toBeDisabled();
      expect(await calls(page, "restore_client_config")).toBe(0);
    }
    expect(await calls(page, "create_image_task")).toBe(0);
    expect(await calls(page, "apply_client_config")).toBe(0);
  });
}

test("Claude import export requires confirmation and never claims configuration was applied", async ({
  page,
}) => {
  const state = bootstrap();
  state.clients[1]!.configuration_state = "not_configured";
  state.clients[1]!.loaded_state = "unconfirmed";
  await fixture(page, { state, claudeImport: true });
  await page.goto("/#/access");
  await page.locator("#model-claude").selectOption("synthetic-text");
  await page.getByRole("button", { name: "预览配置" }).last().click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("button", { name: "确认应用" })).toBeDisabled();
  const exportButton = dialog.getByRole("button", {
    name: "确认并导出 Claude 导入文件",
  });
  await expect(exportButton).toBeEnabled();
  expect(await calls(page, "export_claude_import")).toBe(0);
  await exportButton.click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(
    page.getByText(
      "Claude 导入文件已保存。请手动导入并完整重启，配置载入和连接仍待确认。",
    ),
  ).toBeVisible();
  expect(await calls(page, "apply_client_config")).toBe(0);
  expect(await calls(page, "export_claude_import")).toBe(1);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __ORBIT_TEST__: FixtureControl }).__ORBIT_TEST__
          .lastRequests.export_claude_import,
    ),
  ).toEqual({ preview_id: "synthetic-preview", confirmed: true });
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "首页", exact: true })
    .click();
  await expect(page.getByText("配置已保存", { exact: true })).toHaveCount(1);
  await expect(page.getByText("尚未配置", { exact: true })).toHaveCount(1);
});

test("UTF-8 bound prevents a paid submit and failed paid submission never retries", async ({
  page,
}) => {
  await fixture(page, { paidFailure: true });
  await page.goto("/#/images");
  await page.locator("#image-model").selectOption("synthetic-image");
  await page.locator("#image-prompt").fill("中".repeat(10923));
  await expect(
    page.getByRole("button", { name: "生成图片", exact: true }),
  ).toBeDisabled();
  expect(await calls(page, "create_image_task")).toBe(0);
  await page.locator("#image-prompt").fill("Synthetic image prompt");
  await page.getByRole("button", { name: "生成图片", exact: true }).click();
  await expect(
    page.getByText("结果待确认", { exact: true }).first(),
  ).toBeVisible();
  await expect(page.getByText("费用待确认", { exact: true })).toBeVisible();
  await expect(
    page.getByText("SYNTHETIC_SECRET_SHOULD_NOT_RENDER"),
  ).toHaveCount(0);
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "首页", exact: true })
    .click();
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "图片工作台", exact: true })
    .click();
  expect(await calls(page, "create_image_task")).toBe(1);
  await expect(page.getByRole("button", { name: "重试生成" })).toHaveCount(0);
});

test("crash-recovered local materials expose only a local saving retry", async ({
  page,
}) => {
  const state = bootstrap();
  const recovered = task("unconfirmed");
  recovered.local_save_available = true;
  state.tasks = [recovered];
  await fixture(page, { state });
  await page.goto("/#/images");
  await page.getByRole("button", { name: "重试本地保存" }).click();
  await expect(page.locator(".task-header .badge")).toHaveText(
    "本地结果已保存",
  );
  expect(await calls(page, "retry_local_save")).toBe(1);
  expect(await calls(page, "create_image_task")).toBe(0);
  await expect(page.getByText("费用待确认", { exact: true })).toBeVisible();
});

test("deleting one local image never deletes other results of the same task", async ({
  page,
}) => {
  const state = bootstrap();
  state.tasks = [task()];
  state.tasks[0]!.image_ids = ["image-1", "image-2"];
  state.image_history = ["image-1", "image-2"].map((id) => ({
    id,
    task_id: "synthetic-task",
    account_id: "synthetic-account-a",
    created_at: "2026-10-13",
    requested_model: "synthetic-image",
    observed_model: null,
    mime: "image/png",
    width: 16,
    height: 16,
    bytes: 100,
    preview_url: null,
  }));
  await fixture(page, { state });
  await page.goto("/#/images");
  await page.getByRole("tab", { name: "本地历史" }).click();
  await expect(page.locator(".history-card")).toHaveCount(2);
  await page
    .getByRole("button", { name: "删除本地历史", exact: true })
    .first()
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "确认", exact: true })
    .click();
  await expect(page.locator(".history-card")).toHaveCount(1);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __ORBIT_TEST__: FixtureControl }).__ORBIT_TEST__
          .lastRequests.delete_history,
    ),
  ).toEqual({ image_ids: ["image-1"], confirmed: true });
  expect(await calls(page, "create_image_task")).toBe(0);
});

test("old revocation confirmation is disabled after credential replacement", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/access");
  await page
    .getByRole("button", { name: "撤销本安装推理 key", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(
    dialog.getByRole("button", { name: "确认", exact: true }),
  ).toBeEnabled();
  const changed = bootstrap("synthetic-account-a", 2);
  changed.connections[0]!.revision = 2;
  changed.connections[0]!.credential_revision = "new-credential";
  await pushState(page, changed);
  await expect(
    dialog.getByRole("button", { name: "确认", exact: true }),
  ).toBeDisabled();
  expect(await calls(page, "revoke_inference_key")).toBe(0);
});

test("theme, language, wide window and accessible image tabs", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/settings");
  await page.locator("#setting-theme").selectOption("dark");
  await expect(page.locator("html")).toHaveClass("dark");
  await page.locator("#setting-locale").selectOption("en");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(
    "Make Orbit fit your workflow",
  );
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "Images", exact: true })
    .click();
  const create = page.getByRole("tab", { name: "Create", exact: true });
  await page.screenshot({
    path: ".cache/evidence/images-composer-dark-wide.png",
    fullPage: true,
  });
  await create.focus();
  await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("tab", { name: "Local history" })).toBeFocused();
  await page.screenshot({
    path: ".cache/evidence/images-dark-wide.png",
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});
