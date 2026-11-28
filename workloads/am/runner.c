/* Retain upstream benchmark execution and performance reporting, adding ROI
 * markers and structured per-item results for the host-side validator. */
#define main upstream_main
#include "upstream_bench.c"
#undef main
extern void m5_work_begin(uint64_t id, uint64_t threadid);
extern void m5_work_end(uint64_t id, uint64_t threadid);

/* Avoid upstream format_time's sprintf("%d", uint64_t) width mismatch and
 * preserve the full 64-bit time using the supported freestanding printf. */
static void print_time(uint64_t us) {
  printf("%llu.%c%c%c", (unsigned long long)(us / 1000),
         '0' + (int)(us / 100 % 10), '0' + (int)(us / 10 % 10),
         '0' + (int)(us % 10));
}

int main(const char *args) {
  const char *inputs[] = {"test", "train", "ref", "huge"};
  unsigned setting_id = 0;
  while (setting_id < LENGTH(inputs) && strcmp(args, inputs[setting_id]) != 0)
    ++setting_id;
  if (setting_id == LENGTH(inputs)) halt(2);
  ioe_init();
  printf("======= Running MicroBench [input *%s*] =======\n", args);
  uint32_t bench_score = 0;
  uint64_t t0 = uptime();
  uint64_t score_time = 0;
  int pass = 1;
  for (unsigned i = 0; i < LENGTH(benchmarks); ++i) {
    current = &benchmarks[i];
    setting = &current->settings[setting_id];
    if (bench_check(current) != NULL) {
      printf("AM_RESULT %s skipped\n", current->name);
      halt(3);
    }
    printf("[%s] %s: ", current->name, current->desc);
    uint64_t usec = ULLONG_MAX;
    int succ = 1;
    for (unsigned repeat = 0; repeat < REPEAT; ++repeat) {
      bench_reset();
      current->prepare();
      Result res;
      bench_prepare(&res);
      m5_work_begin(i * REPEAT + repeat + 1, 0);
      current->run();
      m5_work_end(i * REPEAT + repeat + 1, 0);
      bench_done(&res);
      res.pass = current->validate();
      printf(res.pass ? "*" : "X");
      succ &= res.pass;
      if (res.usec < usec) usec = res.usec;
      score_time += res.usec;
    }
    printf(succ ? " Passed.\n" : " Failed.\n");
    uint32_t cur = succ ? score(current, usec) : 0;
    if (setting_id != 0) {
      printf("  min time: ");
      print_time(usec);
      printf(" ms [%u]\n", cur);
    }
    bench_score += cur;
    pass &= succ;
    printf("AM_RESULT %s %s\n", current->name, succ ? "passed" : "failed");
  }
  uint64_t total_time = uptime() - t0;
  bench_score /= LENGTH(benchmarks);
  printf("==================================================\n");
  printf("MicroBench %s", pass ? "PASS" : "FAIL");
  if (setting_id >= 2) {
    printf("        %u Marks\n", bench_score);
    printf("                   vs. %u Marks (%s)\n", REF_SCORE, REF_CPU);
  } else printf("\n");
  printf("Scored time: ");
  print_time(score_time);
  printf(" ms\nTotal  time: ");
  print_time(total_time);
  printf(" ms\n");
  return pass ? 0 : 1;
}
