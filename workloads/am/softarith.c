/* RV64I compiler helpers: unsigned arithmetic avoids signed-overflow UB.
 * No host services and no multiplication/division operators in these helpers. */
#include <stdint.h>
uint64_t __muldi3(uint64_t a, uint64_t b) {
  uint64_t result = 0;
  while (b) {
    if (b & 1) result += a;
    a <<= 1; b >>= 1;
  }
  return result;
}
static uint64_t divide(uint64_t n, uint64_t d, uint64_t *rem) {
  if (d == 0) { *rem = n; return UINT64_MAX; }
  uint64_t q = 0, r = 0;
  for (int bit = 63; bit >= 0; --bit) {
    uint64_t carry = r >> 63;
    r = (r << 1) | ((n >> bit) & 1);
    if (carry || r >= d) { r -= d; q |= (uint64_t)1 << bit; }
  }
  *rem = r; return q;
}
uint64_t __udivdi3(uint64_t n, uint64_t d) { uint64_t r; return divide(n, d, &r); }
uint64_t __umoddi3(uint64_t n, uint64_t d) { uint64_t r; divide(n, d, &r); return r; }
int64_t __divdi3(int64_t n, int64_t d) {
  uint64_t un = (uint64_t)n, ud = (uint64_t)d, r;
  if (!d) return -1;
  uint64_t q = divide(n < 0 ? 0 - un : un, d < 0 ? 0 - ud : ud, &r);
  return (int64_t)((n < 0) != (d < 0) ? 0 - q : q);
}
int64_t __moddi3(int64_t n, int64_t d) {
  uint64_t un = (uint64_t)n, ud = (uint64_t)d, r;
  divide(n < 0 ? 0 - un : un, d < 0 ? 0 - ud : ud, &r);
  return (int64_t)(n < 0 ? 0 - r : r);
}
