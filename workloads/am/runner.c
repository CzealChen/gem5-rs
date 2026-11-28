/* The upstream helpers, prepare/run/validate and inputs are retained unchanged.
 * Only suite selection, ROI markers and structured per-item reporting differ. */
#define main upstream_main
#include "upstream_bench.c"
#undef main
extern void m5_work_begin(uint64_t id, uint64_t threadid);
extern void m5_work_end(uint64_t id, uint64_t threadid);

static Benchmark selected[] = {
  ENTRY(queen, "queen", QUEEN_S, QUEEN_M, QUEEN_L, QUEEN_H, "Queen placement")
  ENTRY(qsort, "qsort", QSORT_S, QSORT_M, QSORT_L, QSORT_H, "Quick sort")
  ENTRY(sieve, "sieve", SIEVE_S, SIEVE_M, SIEVE_L, SIEVE_H, "Eratosthenes sieve")
  ENTRY(md5, "md5", MD5_S, MD5_M, MD5_L, MD5_H, "MD5 digest")
};
int main(const char *args) {
  if (strcmp(args, "test") != 0) halt(2);
  ioe_init();
  for (unsigned i = 0; i < LENGTH(selected); ++i) {
    current = &selected[i];
    setting = &current->settings[0];
    if (bench_check(current) != NULL) {
      printf("AM_RESULT %s skipped\n", current->name);
      halt(3);
    }
    bench_reset();
    current->prepare();
    m5_work_begin(i + 1, 0);
    current->run();
    m5_work_end(i + 1, 0);
    int pass = current->validate();
    printf("AM_RESULT %s %s\n", current->name, pass ? "passed" : "failed");
    if (!pass) halt(4);
  }
  return 0;
}
