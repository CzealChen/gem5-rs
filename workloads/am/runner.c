/* The upstream helpers, prepare/run/validate and inputs are retained unchanged.
 * Only ROI markers and structured per-item reporting differ. */
#define main upstream_main
#include "upstream_bench.c"
#undef main
extern void m5_work_begin(uint64_t id, uint64_t threadid);
extern void m5_work_end(uint64_t id, uint64_t threadid);

int main(const char *args) {
  const char *inputs[] = {"test", "train", "ref", "huge"};
  unsigned setting_id = 0;
  while (setting_id < LENGTH(inputs) && strcmp(args, inputs[setting_id]) != 0)
    ++setting_id;
  if (setting_id == LENGTH(inputs)) halt(2);
  ioe_init();
  for (unsigned i = 0; i < LENGTH(benchmarks); ++i) {
    current = &benchmarks[i];
    setting = &current->settings[setting_id];
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
