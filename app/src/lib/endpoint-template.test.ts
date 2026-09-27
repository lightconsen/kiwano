// The template-endpoint rule, one file because the Hub contract is one rule:
// {placeholder} holes in a catalog URL are the user's to fill at add time,
// and the composed URL is the only thing that may reach the backend.
import { describe, expect, it } from "vitest";

import {
  endpointParams,
  hasUnfilledParams,
  instantiateEndpoint,
  isTemplateEndpoint,
} from "./endpoint-template";

describe("template endpoints", () => {
  it("names the placeholders in order", () => {
    expect(endpointParams("https://bedrock-runtime.{region}.amazonaws.com/openai/v1")).toEqual([
      "region",
    ]);
    expect(endpointParams("https://api.cloudflare.com/client/v4/accounts/{account-id}/ai/v1")).toEqual([
      "account-id",
    ]);
    expect(endpointParams("https://api.deepseek.com")).toEqual([]);
  });

  it("treats a name with dashes and digits as one hole", () => {
    expect(isTemplateEndpoint("https://{resource-name-2}.host")).toBe(true);
    // A capital or a slash is not the Hub's placeholder syntax — it is just
    // text, and the dialog leaves it alone rather than guessing.
    expect(isTemplateEndpoint("https://{Region}.host")).toBe(false);
    expect(isTemplateEndpoint("https://x/{a/b}")).toBe(false);
  });

  it("composes the concrete URL from filled values", () => {
    expect(
      instantiateEndpoint("https://{region}.ml.cloud.ibm.com/ml/v1/text", { region: "us-south" }),
    ).toBe("https://us-south.ml.cloud.ibm.com/ml/v1/text");
  });

  it("leaves unfilled holes visible instead of inventing a value", () => {
    expect(instantiateEndpoint("https://bedrock-runtime.{region}.amazonaws.com", {})).toBe(
      "https://bedrock-runtime.{region}.amazonaws.com",
    );
    expect(
      instantiateEndpoint("https://{r1}/{r2}", { r1: "a" }),
    ).toBe("https://a/{r2}");
  });

  it("reports a template incomplete until every hole is filled", () => {
    expect(hasUnfilledParams("https://bedrock-runtime.{region}.amazonaws.com", {})).toBe(true);
    expect(hasUnfilledParams("https://bedrock-runtime.{region}.amazonaws.com", { region: " " })).toBe(
      true,
    );
    expect(
      hasUnfilledParams("https://bedrock-runtime.{region}.amazonaws.com", { region: "eu-west-1" }),
    ).toBe(false);
    expect(hasUnfilledParams("https://api.deepseek.com", {})).toBe(false);
  });
});
