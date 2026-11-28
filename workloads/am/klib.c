/* Required freestanding klib subset. Unsupported formatting fails explicitly. */
#include <klib.h>
void *memset(void *dst, int c, size_t n) {
  unsigned char *p = dst;
  for (size_t i = 0; i < n; ++i) p[i] = (unsigned char)c;
  return dst;
}
void *memcpy(void *dst, const void *src, size_t n) {
  unsigned char *d = dst; const unsigned char *s = src;
  for (size_t i = 0; i < n; ++i) d[i] = s[i];
  return dst;
}
int strcmp(const char *a, const char *b) {
  while (*a && *a == *b) { ++a; ++b; }
  return (unsigned char)*a - (unsigned char)*b;
}
size_t strlen(const char *s) { size_t n = 0; while (s[n]) ++n; return n; }
static int number(uint64_t value, unsigned base) {
  char buf[64]; int n = 0, count = 0;
  do { buf[n++] = "0123456789abcdef"[value % base]; value /= base; } while (value);
  count = n;
  while (n) putch(buf[--n]);
  return count;
}
int vprintf(const char *fmt, va_list args) {
  int count = 0;
  while (*fmt) {
    if (*fmt != '%') { putch(*fmt++); ++count; continue; }
    ++fmt;
    unsigned length = 0;
    while (*fmt == 'l' && length < 2) { ++length; ++fmt; }
    char spec = *fmt++;
    if (spec == '%' && !length) { putch('%'); ++count; }
    else if (spec == 's' && !length) {
      const char *s = va_arg(args, const char *);
      while (*s) { putch(*s++); ++count; }
    } else if (spec == 'c' && !length) { putch(va_arg(args, int)); ++count; }
    else if (spec == 'd' || spec == 'u' || spec == 'x') {
      uint64_t value;
      if (spec == 'd') {
        int64_t signed_value = length == 2 ? va_arg(args, long long) : length ? va_arg(args, long) : va_arg(args, int);
        value = (uint64_t)signed_value;
        if (signed_value < 0) { putch('-'); ++count; value = 0 - value; }
      } else value = length == 2 ? va_arg(args, unsigned long long) : length ? va_arg(args, unsigned long) : va_arg(args, unsigned);
      count += number(value, spec == 'x' ? 16 : 10);
    } else panic("unsupported printf format");
  }
  return count;
}
int printf(const char *fmt, ...) {
  va_list args; va_start(args, fmt);
  int count = vprintf(fmt, args);
  va_end(args); return count;
}
