/// 表单密码生成（从 VaultFormView 抽出以便单测）：
/// crypto 随机 16 位，去掉易混淆字符（i l o I L O 0 1）
const CHARS = "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*";

export function generatePassword(): string {
  const arr = new Uint32Array(16);
  crypto.getRandomValues(arr);
  return Array.from(arr, (n) => CHARS[n % CHARS.length]).join("");
}
