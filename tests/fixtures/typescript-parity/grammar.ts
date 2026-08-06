// @ts-expect-error the next-line form
const a: number = "one";
/* @ts-ignore the single-line block form */
const b: number = "two";
/** @ts-expect-error the JSDoc block form */
const c: number = "three";
/* prose first, and then
   @ts-ignore on the comment's last line */
const d: number = "four";
