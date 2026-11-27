static volatile unsigned long inputs[] = {1, 3, 5, 7};
static volatile unsigned long result;

unsigned long term(unsigned long value, unsigned long index);

int workload_main(void)
{
    if (result != 0)
        return 1;

    for (unsigned long i = 0; i < 4; ++i)
        result += term(inputs[i], i);

    return result == 38 ? 0 : 2;
}
