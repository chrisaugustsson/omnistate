import { expect, test } from "vitest";
import { healthResponse } from "./health";

test("the health response is 200", () => {
  const response = healthResponse();
  expect(response.status).toBe(200);
});
