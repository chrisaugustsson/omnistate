import { describe, expect, test } from "vitest";
import { readSystem, systemTokens, tokenValues } from "./system";

describe("design-system token values", () => {
  test("inherits token types and resolves chained and composite aliases in the selected mode", () => {
    const tokens = systemTokens({
      $extensions: { source: { ignored: { $value: "metadata" } } },
      color: {
        $type: "color",
        surface: { $value: "#ffffff", $extensions: { omnistate: { modes: { dark: "#171717" } } } },
        background: { $value: "{color.surface}" },
        overlay: { $value: "{color.background}" },
      },
      spacing: { unit: { $type: "dimension", $value: "4px" } },
      shadow: { card: { $type: "shadow", $value: { color: "{color.overlay}", offsetX: "0px", offsetY: "{spacing.unit}", blur: "8px", spread: "0px" } } },
    });
    expect(tokens).toHaveLength(5);
    expect(tokens.find((token) => token.path === "color.overlay")?.type).toBe("color");
    expect(tokenValues(tokens, "light").get("color.overlay")).toBe("#ffffff");
    expect(tokenValues(tokens, "dark").get("shadow.card")).toEqual({ color: "#171717", offsetX: "0px", offsetY: "4px", blur: "8px", spread: "0px" });
  });

  test("an unresolved or cyclic reference does not stop valid tokens from being shown", () => {
    const tokens = systemTokens({ color: { $type: "color", good: { $value: "#123456" }, missing: { $value: "{color.absent}" }, a: { $value: "{color.b}" }, b: { $value: "{color.a}" } } });
    const values = tokenValues(tokens, null);
    expect(values.get("color.good")).toBe("#123456");
    expect(values.get("color.missing")).toBeNull();
    expect(values.get("color.a")).toBeNull();
    expect(values.get("color.b")).toBeNull();
  });

  test("an empty head is an empty catalog and a non-object payload is rejected", () => {
    expect(systemTokens(readSystem(""))).toEqual([]);
    expect(() => readSystem("[]")).toThrow();
  });
});
