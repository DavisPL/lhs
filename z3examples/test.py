import z3
s_0 = z3.Solver()
f_0 = z3.String("f_0")
n_0 = z3.Int("n_0")

s_0.add(n_0 > 0)
f_1 = z3.String("f_1")
# s_0.add(z3.Concat(f_0, f_1))
s_0.add(f_1 == z3.Concat(f_0, "/mem"))

# n_1 = z3.Int("n_1")
s_0.add(n_0 + 5 > 10)
s_0.add(f_1 == "/prof/self/mem")

if s_0.check() == z3.sat:
    print("The formula is satisfiable.")
    model = s_0.model()
    print("Model:")
    print(model)
else:
    print("The formula is not satisfiable.")
