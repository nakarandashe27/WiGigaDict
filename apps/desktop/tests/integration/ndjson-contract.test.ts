import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { PROTOCOL_VERSION } from "../../src/lib/version";

const fixtureDirectory = fileURLToPath(
  new URL("../../../../tests/contracts/ndjson/", import.meta.url),
);

const readFixture = (name: string): Record<string, unknown> => {
  const text = readFileSync(`${fixtureDirectory}/${name}`, "utf8");
  const lines = text.split(/\r?\n/u).filter((line) => line.length > 0);
  expect(lines).toHaveLength(1);
  return JSON.parse(lines[0]) as Record<string, unknown>;
};

const expectedKeys = ["app", "commit", "protocol", "role", "type"];

describe("shared NDJSON fixtures", () => {
  it("describes the shell hello contract", () => {
    const hello = readFixture("hello.valid.ndjson");

    expect(Object.keys(hello).sort()).toEqual(expectedKeys);
    expect(hello).toMatchObject({
      type: "hello",
      protocol: PROTOCOL_VERSION,
      role: "shell",
    });
  });

  it("describes the sidecar acknowledgement contract", () => {
    const ack = readFixture("hello-ack.valid.ndjson");

    expect(Object.keys(ack).sort()).toEqual(expectedKeys);
    expect(ack).toMatchObject({
      type: "hello_ack",
      protocol: PROTOCOL_VERSION,
      role: "sidecar",
    });
  });

  it("keeps incompatible and unknown-field fixtures visibly invalid", () => {
    const wrongProtocol = readFixture("hello.wrong-protocol.ndjson");
    const unknownField = readFixture("hello.unknown-field.ndjson");

    expect(wrongProtocol.protocol).not.toBe(PROTOCOL_VERSION);
    expect(Object.keys(unknownField).sort()).not.toEqual(expectedKeys);
  });
});
