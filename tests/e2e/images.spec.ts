import { expect, test } from "@playwright/test";
import { bootstrap, task } from "../fixtures";
import { apiKeyPage } from "../api-key-fixtures";
import { calls, fixture, pushState } from "./native-fixture";

test("creation previews survive navigation but are empty in a new app session with the same saved history", async ({
  page,
  context,
}) => {
  const state = bootstrap();
  await fixture(page, { state });
  await context.route("http://asset.localhost/**", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#cab8a4"/></svg>',
    }),
  );
  await page.goto("/#/images");
  await page.locator("#image-model").click();
  await page
    .getByRole("option", { name: "synthetic-image", exact: true })
    .click();
  await page.locator("#image-prompt").fill("Current session preview");
  await page.getByRole("button", { name: "生成图片", exact: true }).click();
  await expect(page.locator(".task-header")).toBeVisible();
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "首页", exact: true })
    .click();

  const completed = { ...task(), image_ids: ["session-image"] };
  state.tasks = [completed];
  state.image_history = [
    {
      id: "session-image",
      task_id: completed.id,
      account_id: completed.account_id,
      created_at: "2026-11-16T03:00:00Z",
      requested_model: completed.requested_model,
      observed_model: null,
      mime: "image/png",
      width: 16,
      height: 16,
      bytes: 100,
      preview_url: "http://asset.localhost/session-image",
    },
  ];
  state.state_revision = 10;
  await pushState(page, state);
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "图片工作台", exact: true })
    .click();
  await expect(page.locator(".result-card img")).toBeVisible();
  await page.getByRole("tab", { name: "本地历史" }).click();
  await expect(page.locator(".history-card")).toHaveCount(1);
  await page.getByRole("tab", { name: "创作", exact: true }).click();
  await expect(page.locator(".result-card img")).toBeVisible();
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "首页", exact: true })
    .click();
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "图片工作台", exact: true })
    .click();
  await expect(page.locator(".result-card img")).toBeVisible();
  state.state_revision++;
  await pushState(page, state);
  await expect(page.locator(".result-card img")).toBeVisible();
  expect(await calls(page, "create_image_task")).toBe(1);

  // A new webview receives the same persisted native payload after app restart.
  const reopened = await context.newPage();
  await fixture(reopened, { state });
  await reopened.goto("/#/images");
  await expect(reopened.locator(".task-header")).toHaveCount(0);
  await expect(reopened.locator(".result-card")).toHaveCount(0);
  await expect(reopened.locator(".image-result")).toContainText(
    "结果将在这里预览",
  );
  await reopened.getByRole("button", { name: "刷新数据", exact: true }).click();
  await expect(reopened.locator(".result-card")).toHaveCount(0);
  await reopened.getByRole("tab", { name: "本地历史" }).click();
  await expect(reopened.locator(".history-card img")).toBeVisible();
  expect(await calls(reopened, "create_image_task")).toBe(0);

  // Interrupted saves can also contain images that were written before shutdown.
  state.tasks = [
    { ...completed, state: "unconfirmed", local_save_available: true },
  ];
  const recovered = await context.newPage();
  await fixture(recovered, { state });
  await recovered.goto("/#/images");
  await expect(recovered.locator(".task-header")).toContainText("结果待确认");
  await expect(recovered.locator(".result-card")).toHaveCount(0);
  await recovered.getByRole("button", { name: "重试本地保存" }).click();
  await expect(recovered.locator(".result-card img")).toBeVisible();
  expect(await calls(recovered, "retry_local_save")).toBe(1);
  expect(await calls(recovered, "create_image_task")).toBe(0);
});

test("local references and history editing work before key selection and keep the draft until submission", async ({
  page,
}) => {
  const state = bootstrap();
  state.session.inference_credential_status = "unavailable";
  state.connections[0]!.credential_revision = null;
  const completed = { ...task(), image_ids: ["saved-image"] };
  state.tasks = [completed];
  state.image_history = [
    {
      id: "saved-image",
      task_id: completed.id,
      account_id: completed.account_id,
      created_at: "2026-11-17T03:00:00Z",
      requested_model: completed.requested_model,
      observed_model: null,
      mime: "image/png",
      width: 1536,
      height: 1024,
      bytes: 1992294,
      preview_url: null,
    },
  ];
  const keys = apiKeyPage();
  keys.selected = null;
  keys.selected_ready = false;
  await fixture(page, {
    state,
    apiKeys: keys,
    imageInputs: [
      {
        id: "reference",
        name: "reference.png",
        mime: "image/png",
        width: 16,
        height: 16,
        bytes: 100,
        preview_url: null,
      },
    ],
  });
  await page.goto("/#/images");
  await page.locator("#image-model").click();
  await page
    .getByRole("option", { name: "synthetic-image", exact: true })
    .click();
  await page
    .locator("#image-prompt")
    .fill("Draft prepared before choosing a key");
  await expect(page.locator("#image-submit-reason")).toContainText(
    "请先选择有效的 API Key",
  );
  await expect(
    page.getByRole("button", { name: "添加参考图", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "添加参考图", exact: true }).click();
  await expect(page.locator(".input-images")).toContainText("reference.png");
  await page.getByRole("tab", { name: "本地历史" }).click();
  await page
    .locator(".history-card")
    .getByRole("button", { name: "再次编辑", exact: true })
    .click();
  await expect(page.locator(".input-images")).toContainText("saved-image.png");
  await expect(
    page.getByRole("button", { name: "编辑图片", exact: true }),
  ).toBeDisabled();
  expect(await calls(page, "create_image_task")).toBe(0);
  await page
    .getByRole("button", { name: "编辑图片", exact: true })
    .scrollIntoViewIfNeeded();
  await expect(page.locator("#image-submit-reason")).toBeInViewport();
  await page
    .locator(".image-submit-actions")
    .screenshot({ path: ".cache/evidence/images-key-required.png" });
  await page
    .locator("#image-submit-reason")
    .getByRole("link", { name: "选择 API Key" })
    .click();
  await page.getByRole("button", { name: "切换密钥", exact: true }).click();
  await page.getByRole("radio").first().check();
  await page.getByRole("button", { name: "使用此密钥", exact: true }).click();
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "图片工作台", exact: true })
    .click();
  await expect(page.locator("#image-prompt")).toHaveValue(
    "Draft prepared before choosing a key",
  );
  await expect(page.locator(".input-images")).toContainText("saved-image.png");
  await expect(
    page.getByRole("button", { name: "编辑图片", exact: true }),
  ).toBeEnabled();
  expect(await calls(page, "refresh_image_inputs")).toBe(1);
  await page.getByRole("button", { name: "编辑图片", exact: true }).click();
  expect(await calls(page, "create_image_task")).toBe(1);
});

test("history titles use request IDs, truncate within the card and support persistent rename and reset", async ({
  page,
}) => {
  const state = bootstrap();
  const completed = {
    ...task(),
    request_id: "request-" + "0123456789".repeat(15),
    image_ids: ["saved-image"],
  };
  state.tasks = [completed];
  state.image_history = [
    {
      id: "saved-image",
      task_id: completed.id,
      account_id: completed.account_id,
      created_at: "2026-11-17T03:00:00Z",
      requested_model: completed.requested_model,
      observed_model: null,
      mime: "image/png",
      width: 1536,
      height: 1024,
      bytes: 1992294,
      preview_url: null,
    },
  ];
  await fixture(page, { state });
  await page.goto("/#/images");
  await page.getByRole("tab", { name: "本地历史" }).click();
  const card = page.locator(".history-card");
  const title = card.locator("strong");
  await expect(title).toHaveText(completed.request_id);
  await expect(title).toHaveAttribute("title", completed.request_id);
  await expect(card.locator(".history-card-info > p").first()).toHaveText(
    "PNG · 1536 × 1024 · 1.9 MiB",
  );
  for (const width of [1280, 460]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(title).toHaveCSS("text-overflow", "ellipsis");
    expect(
      await title.evaluate((node) => node.scrollWidth > node.clientWidth),
    ).toBe(true);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
  }
  await card.getByRole("button", { name: "修改标题", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "修改标题", exact: true });
  await dialog.locator("#image-history-title").fill("海报草稿");
  await dialog.getByRole("button", { name: "保存更改", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(title).toHaveText("海报草稿");
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "首页", exact: true })
    .click();
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "图片工作台", exact: true })
    .click();
  await page.getByRole("tab", { name: "本地历史" }).click();
  await expect(title).toHaveText("海报草稿");
  await card.getByRole("button", { name: "修改标题", exact: true }).click();
  await dialog.locator("#image-history-title").fill("");
  await dialog.getByRole("button", { name: "保存更改", exact: true }).click();
  await expect(title).toHaveText(completed.request_id);
  expect(await calls(page, "rename_history_image")).toBe(2);
  expect(await calls(page, "create_image_task")).toBe(0);
  await card.screenshot({
    path: ".cache/evidence/images-history-request-title.png",
  });
});

test("submission explains missing model and prompt and unavailable nodes next to the button", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/images");
  await expect(page.locator("#image-submit-reason")).toHaveText(
    "请选择图片模型。",
  );
  await page.locator("#image-model").click();
  await page
    .getByRole("option", { name: "synthetic-image", exact: true })
    .click();
  await expect(page.locator("#image-submit-reason")).toHaveText(
    "请填写提示词，描述要生成的图片或修改要求。",
  );
  await page.locator("#image-prompt").fill("A synthetic draft");
  await expect(page.locator("#image-submit-reason")).toHaveCount(0);
  const changed = bootstrap("synthetic-account-a", 2);
  changed.connections[0]!.node_id = null;
  await pushState(page, changed);
  await expect(page.locator("#image-submit-reason")).toContainText(
    "请先选择可用节点。",
  );
  await expect(
    page.getByRole("button", { name: "添加参考图", exact: true }),
  ).toBeEnabled();
  expect(await calls(page, "create_image_task")).toBe(0);
  await page
    .getByRole("button", { name: "生成图片", exact: true })
    .scrollIntoViewIfNeeded();
  await expect(page.locator("#image-submit-reason")).toBeInViewport();
  await page
    .locator(".image-submit-actions")
    .screenshot({ path: ".cache/evidence/images-submit-reason.png" });
});

test("recovered image tasks without a catalog entry do not fall back to a model ID", async ({
  page,
}) => {
  const state = bootstrap();
  state.tasks = [task("unconfirmed")];
  await fixture(page, { state, models: [] });
  await page.goto("/#/images");
  await expect(page.locator(".task-facts dt")).toHaveText(["Request ID"]);
  await expect(page.locator(".image-result")).not.toContainText(
    "synthetic-image",
  );
});

test("a task keeps its saved model label after the model leaves the catalog", async ({
  page,
}) => {
  const state = bootstrap();
  state.tasks = [
    { ...task("unconfirmed"), requested_model_label: "创建时的模型名称" },
  ];
  await fixture(page, { state, models: [] });
  await page.goto("/#/images");
  await expect(page.locator(".task-facts dt")).toHaveText([
    "请求模型",
    "Request ID",
  ]);
  await expect(page.locator(".task-facts dd").first()).toHaveText(
    "创建时的模型名称",
  );
  await expect(page.locator(".image-result")).not.toContainText(
    "synthetic-image",
  );
});
