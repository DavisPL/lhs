import z3

############ case 1 #############

s_0 = z3.Solver()
f_0 = z3.String("f_0")
n_0 = z3.Int("n_0")

s_0.add(n_0 > 0)
f_1 = z3.String("f_1")
s_0.add(f_1 == z3.Concat(f_0, "/mem"))

s_0.add(n_0 + 5 > 10)
s_0.add(f_1 == "/prof/self/mem")

if s_0.check() == z3.sat:
    print("The formula is satisfiable.")
    model = s_0.model()
    print("Model:")
    print(model)
else:
    print("The formula is not satisfiable.")

############ case 2 #############

s_1 = z3.Solver()
f_2 = z3.String("f_2")
n_1 = z3.Int("n_1")

s_1.add(n_1 > 0)
f_3 = z3.String("f_3")
s_1.add(f_3 == z3.Concat(f_2, "/mem"))

s_1.add(z3.Not(n_1 + 5 > 10))
s_1.add(False)

############ case 3 #############

s_2 = z3.Solver()
f_4 = z3.String("f_4")
n_2 = z3.Int("n_2")

s_2.add(z3.Not(n_2 > 0))
f_5 = z3.String("f_5")
s_2.add(f_5 == z3.Concat(f_4, "/self"))

s_2.add(f_5 == "/prof/self/mem")

if s_2.check() == z3.sat:
    print("The formula is satisfiable.")
    model = s_2.model()
    print("Model:")
    print(model)
else:
    print("The formula is not satisfiable.")

############ case 4 #############

s_3 = z3.Solver()
f_6 = z3.String("f_6")
n_3 = z3.Int("n_3")

s_3.add(z3.Not(n_3 > 0))
f_7 = z3.String("f_7")
s_3.add(f_7 == z3.Concat(f_6, "/self"))
s_3.add(False)

