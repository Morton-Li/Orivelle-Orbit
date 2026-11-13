import { expect, test } from "@playwright/test";
import { bootstrap, imageModel, task } from "../fixtures";
import { fixture, pushState } from "./native-fixture";

test("image results show the model label and keep each image's natural proportions", async ({
  page,
}) => {
  const state = bootstrap();
  const pending = task("waiting");
  pending.parameters = { size: "1536x1024", quality: "high" };
  state.tasks = [pending];
  await fixture(page, {
    state,
    models: [{ ...imageModel(), label: "图像创作模型" }],
  });
  await page.route("http://asset.localhost/**", (route) => {
    const [width, height] = new URL(route.request().url()).pathname
      .slice(1)
      .split("x");
    return route.fulfill({
      contentType: "image/svg+xml",
      body: `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"><rect width="100%" height="100%" fill="#cab8a4"/></svg>`,
    });
  });
  await page.goto("/#/images");
  await expect(page.locator(".task-facts dt")).toHaveText([
    "请求模型",
    "Request ID",
  ]);
  await expect(page.locator(".task-facts dd").first()).toHaveText(
    "图像创作模型",
  );
  await expect(page.locator(".image-result")).not.toContainText(
    "没有服务端阶段事件",
  );

  for (const [width, height] of [
    [1536, 1024],
    [1024, 1024],
    [1024, 1536],
  ]) {
    const image = {
      id: `image-${width}x${height}`,
      task_id: pending.id,
      account_id: pending.account_id,
      created_at: "2026-11-13T07:30:00Z",
      requested_model: pending.requested_model,
      observed_model: null,
      mime: "image/png" as const,
      width: width!,
      height: height!,
      bytes: 153600,
      preview_url: `http://asset.localhost/${width}x${height}`,
    };
    state.tasks[0] = { ...pending, state: "completed", image_ids: [image.id] };
    state.image_history = [image];
    state.state_revision++;
    await pushState(page, state);
    const preview = page.locator(".result-card .image-preview-button");
    await expect(preview.locator("img")).toHaveJSProperty(
      "naturalWidth",
      width,
    );
    const frame = await preview.boundingBox();
    const content = await preview.locator("img").boundingBox();
    expect(frame!.width / frame!.height).toBeCloseTo(width! / height!, 2);
    expect(content!.height).toBeCloseTo(frame!.height, 0);
    await expect(page.locator(".task-facts dd").first()).toHaveText(
      "图像创作模型",
    );
    await expect(
      page.getByRole("button", { name: "复用参数", exact: true }),
    ).toHaveCount(0);
    await preview.click();
    const enlarged = await page.locator(".zoom-image").boundingBox();
    expect(enlarged!.width / enlarged!.height).toBeCloseTo(width! / height!, 2);
    await page.keyboard.press("Escape");
    await page.getByRole("tab", { name: "本地历史" }).click();
    await expect(page.locator(".history-card strong")).toHaveText(
      `${width} × ${height}`,
    );
    await expect(page.locator(".history-card")).toContainText("PNG · 150 KiB");
    await expect(page.locator(".history-card")).not.toContainText(
      /synthetic-image|实际型号|请求参数|上游返回参数/,
    );
    const thumbnail = await page
      .locator(".history-card .image-preview-button")
      .boundingBox();
    expect(thumbnail!.width / thumbnail!.height).toBeCloseTo(
      width! / height!,
      2,
    );
    await page.getByRole("tab", { name: "创作", exact: true }).click();
  }
});

test("old image tasks without a catalog entry do not fall back to a model ID", async ({
  page,
}) => {
  const state = bootstrap();
  state.tasks = [task()];
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
  state.tasks = [{ ...task(), requested_model_label: "创建时的模型名称" }];
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
