// The SDK owns invoking the locked analyzer and comparing the registrations.
import { check } from "@onebudgetspec/sdk";

try {
  const result = await check({
    paths: [".github/budgets.yaml"],
    cwd: process.cwd(),
  });
  for (const budget of result.results)
    console.log(
      `${budget.id}: ${budget.verdict} (${budget.actual}/${budget.threshold})`,
    );
  if (result.results.some((budget) => budget.verdict !== "within"))
    process.exitCode = 1;
} catch (error) {
  console.error(
    `github-workflows budgets: ${error.message}\nACTION: run github-workflows:test and inspect budget registration/telemetry`,
  );
  process.exitCode = 1;
}
