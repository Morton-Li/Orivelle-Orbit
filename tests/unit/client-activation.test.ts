import { beforeEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import ClientRow from "../../src/components/ClientRow.vue";
import { clientStatus } from "../../src/shared/format";
import { locale } from "../../src/shared/i18n";
import { bootstrap, client } from "../fixtures";

beforeEach(() => {
  locale.value = "zh-CN";
});
describe.each(["codex", "claude"] as const)(
  "%s local connection status",
  (kind) => {
    it("follows native start/restart events without claiming network validation", async () => {
      const target = client(kind);
      const connection = bootstrap().connections[0]!;
      target.activation_state = "waiting_restart";
      const wrapper = mount(ClientRow, {
        props: { client: target, connection, detected: true },
        global: { stubs: { RouterLink: { template: "<a><slot /></a>" } } },
      });
      expect(wrapper.text()).toContain("待重启");
      for (const [state, text, tone] of [
        ["waiting_start", "待启动", "warning"],
        ["connected", "已连接", "success"],
        ["unverified", "运行状态待确认", "warning"],
        ["unavailable", "接入暂不可用", "warning"],
      ] as const) {
        const next = { ...target, activation_state: state };
        await wrapper.setProps({ client: next });
        expect(clientStatus(next, connection, true)).toEqual({ text, tone });
        expect(wrapper.text()).toContain(text);
        expect(wrapper.text()).not.toContain("连接未验证");
        expect(wrapper.text()).not.toContain("连接验证通过");
        expect(wrapper.text()).not.toContain("客户端载入已确认");
      }
      expect(target.loaded_evidence).toBeNull();
      expect(target.connection_evidence).toBeNull();
      wrapper.unmount();
    });
    it("lets conflict, detection and disconnection override an old connected hint", () => {
      const target = {
        ...client(kind),
        activation_state: "connected" as const,
      };
      const connection = bootstrap().connections[0]!;
      expect(
        clientStatus(
          { ...target, configuration_state: "conflict" },
          connection,
          true,
        ).text,
      ).toBe("配置冲突");
      expect(
        clientStatus(
          { ...target, configuration_state: "not_configured" },
          connection,
          true,
        ).text,
      ).toBe("尚未配置");
      expect(
        clientStatus({ ...target, installed: false }, connection, true).text,
      ).toBe("未检测到应用");
      expect(clientStatus(target, connection, false).text).toBe("待检测");
    });
  },
);
