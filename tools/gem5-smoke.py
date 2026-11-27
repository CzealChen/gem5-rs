import m5
from m5.objects import Root


root = Root(full_system=False)
m5.instantiate()
start = m5.curTick()
event = m5.simulate(1000)
end = m5.curTick()
cause = event.getCause()

print(f"gem5 ticks: {start} -> {end} ({cause})")
assert (start, end, cause) == (0, 1000, "simulate() limit reached")
