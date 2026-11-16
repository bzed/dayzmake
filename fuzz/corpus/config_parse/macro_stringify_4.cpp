#define Q(x) #x
#define D(x) x x
#define S1(x) Q(D(x))
#define L0(x) S1(x)
#define L1(x) L0(L0(x))
#define L2(x) L1(L1(x))
#define L3(x) L2(L2(x))
#define L4(x) L3(L3(x))
a = L4(ab);
