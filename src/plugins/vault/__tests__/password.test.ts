import { describe, expect, it } from "vitest";
import { generatePassword } from "../password";

const CHARS =
  "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*";

describe("generatePassword（表单密码生成器）", () => {
  it("长度恒为 16", () => {
    for (let i = 0; i < 20; i++) {
      expect(generatePassword()).toHaveLength(16);
    }
  });

  it("字符全部来自约定字符集（不含易混淆的 i l o I L O 0 1）", () => {
    for (let i = 0; i < 20; i++) {
      const pw = generatePassword();
      for (const c of pw) {
        expect(CHARS.includes(c), `字符 ${c} 不在字符集`).toBe(true);
      }
    }
  });

  it("随机性：多次生成结果互不相同", () => {
    const seen = new Set(Array.from({ length: 20 }, () => generatePassword()));
    expect(seen.size).toBe(20);
  });

  it("含大写、小写与符号三类（抽样下必然齐备）", () => {
    const all = Array.from({ length: 30 }, () => generatePassword()).join("");
    expect(/[a-z]/.test(all)).toBe(true);
    expect(/[A-Z]/.test(all)).toBe(true);
    expect(/[!@#$%^&*]/.test(all)).toBe(true);
  });
});
