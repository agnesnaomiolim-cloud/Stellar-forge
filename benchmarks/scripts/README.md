# Benchmark Scripts

This folder contains the Python utilities that support the performance benchmarking suite. They evaluate k6 results against absolute SLO budgets, compare runs with stored baselines, generate HTML reports, and test the budget gate itself. The scripts are used by the benchmark and regression workflows and can also be run locally.

## Files

| File | Purpose |
| --- | --- |
| `check_performance_budgets.py` | Checks a k6 results JSON file against the blocking and advisory SLO budgets in `benchmarks/performance-budgets.yaml`, with text, JSON, and Markdown output. |
| `compare_benchmarks.py` | Compares current benchmark metrics with a baseline to detect regressions and can create a new baseline file from a run. |
| `generate_report.py` | Generates a standalone HTML report from benchmark results and the comparison JSON produced by `compare_benchmarks.py`. |
| `test_check_performance_budgets.py` | Unit tests for `check_performance_budgets.py`, covering metric lookup, budget comparisons, blocking and advisory behavior, reporting, history, and CLI exit codes. |

## Related documentation

- [Benchmarking suite overview](../README.md)
- [Regression testing guide](../REGRESSION_TESTING.md)
- [Performance budgets](../performance-budgets.yaml)
