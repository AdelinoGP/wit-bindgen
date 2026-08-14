//@ [lang]
//@ path = "stubs/world.ts"

import * as component from "../imports/foo$bar$component";

// The interface's variant, enum, and record are unreferenced by any function.
// `--generate-unused-types` must still emit them, and they must type-check.
const _unused: component.UnusedRecord = new component.UnusedRecord(1);
const _unusedEnum: component.UnusedEnum = component.UnusedEnum.Unused;
const _unusedVariant: component.UnusedVariant = new component.UnusedVariant_Enum(
  component.UnusedEnum.Unused,
);

export function run(): void {
  component.foo();
}
