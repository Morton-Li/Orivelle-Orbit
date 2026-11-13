import { beforeEach, describe, expect, it } from "vitest";
import {
  clientStatus,
  evidenceMatches,
  formatDecimal,
  formatMoney,
  safePreviewUrl,
  utf8Bytes,
} from "../../src/shared/format";
import { locale } from "../../src/shared/i18n";
import { bootstrap, client } from "../fixtures";

beforeEach(() => {
  locale.value = "zh-CN";
});
describe("authoritative financial presentation", () => {
  it("preserves every decimal digit above JavaScript precision without rounding", () => {
    expect(formatDecimal("12345678901234567890123456.123456789012345678")).toBe(
      "12,345,678,901,234,567,890,123,456.123456789012345678",
    );
    expect(formatDecimal("0000.0000")).toBe("0.00");
    expect(formatDecimal("-0.00000001")).toBe("-0.00000001");
    expect(formatDecimal("1e9")).toBeNull();
  });
  it("keeps true zero, null, unknown currency, pending and conflicts distinct", () => {
    expect(formatMoney({ amount: "0", currency: "USD", status: "known" })).toBe(
      "USD 0.00",
    );
    expect(
      formatMoney({ amount: null, currency: "USD", status: "known" }),
    ).toBe("未知");
    expect(formatMoney({ amount: "0", currency: null, status: "known" })).toBe(
      "未知 0.00",
    );
    expect(
      formatMoney({ amount: "0", currency: "USD", status: "pending" }),
    ).toBe("待确认");
    expect(
      formatMoney({ amount: "2", currency: "USD", status: "conflict" }),
    ).toBe("记录冲突");
    expect(
      formatMoney({ amount: null, currency: null, status: "unlimited" }),
    ).toBe("不限");
  });
});
describe("client evidence does not become a false success", () => {
  it("requires the observed configuration, application, actor, connection revision and node", () => {
    const connection = bootstrap().connections[0]!;
    const installed = client();
    const evidence = {
      kind: "fake_gateway_request" as const,
      observed_at: "2026-10-13T03:00:00Z",
      configuration_revision: "config-1",
      client_version: "synthetic-version",
      account_id: connection.account_id,
      connection_id: connection.id,
      connection_revision: connection.revision,
      node_id: "node-a",
      model_id: "synthetic-text",
    };
    installed.connection_state = "passed";
    installed.connection_evidence = evidence;
    expect(clientStatus(installed, connection, true)).toEqual({
      text: "连接验证通过",
      tone: "success",
    });
    for (const changed of [
      { account_id: "another" },
      { revision: 2 },
      { node_id: "other" },
    ]) {
      expect(
        evidenceMatches(evidence, installed, { ...connection, ...changed }),
      ).toBe(false);
      expect(
        clientStatus(installed, { ...connection, ...changed }, true).tone,
      ).not.toBe("success");
    }
    expect(
      evidenceMatches(
        evidence,
        { ...installed, version: "new-version" },
        connection,
      ),
    ).toBe(false);
    expect(
      evidenceMatches(
        evidence,
        { ...installed, configuration_revision: "restored" },
        connection,
      ),
    ).toBe(false);
    expect(clientStatus(installed, connection, false)).toEqual({
      text: "待检测",
      tone: "neutral",
    });
  });
  it("saved configuration without runtime evidence stays unconfirmed", () => {
    const installed = client();
    installed.loaded_state = "confirmed";
    installed.connection_state = "passed";
    expect(clientStatus(installed, bootstrap().connections[0]!, true)).toEqual({
      text: "待重启 / 载入未确认",
      tone: "warning",
    });
  });
});
describe("IPC-facing bounds", () => {
  it("counts UTF-8 rather than characters at the request prompt boundary", () => {
    expect(utf8Bytes("中".repeat(10922))).toBe(32766);
    expect(utf8Bytes("中".repeat(10923))).toBe(32769);
    expect(utf8Bytes("😀".repeat(8192))).toBe(32768);
  });
  it("keeps images on the scoped native asset protocol", () => {
    expect(safePreviewUrl("asset://localhost/fixture/thumb.png")).toBe(
      "asset://localhost/fixture/thumb.png",
    );
    expect(safePreviewUrl("http://asset.localhost/fixture/thumb.png")).toBe(
      "http://asset.localhost/fixture/thumb.png",
    );
    for (const value of [
      "https://edge.invalid/private.png",
      "data:image/png;base64,AAAA",
      "file:///etc/passwd",
      "asset://other/secret",
      "https://user:secret@asset.localhost/image",
      "asset://localhost/image?token=secret",
    ])
      expect(safePreviewUrl(value)).toBeNull();
  });
});
