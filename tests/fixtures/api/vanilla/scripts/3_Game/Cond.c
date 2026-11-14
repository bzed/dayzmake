#ifdef DIAG_DEVELOPER
//! Diag only.
class DiagOnly extends Base
{
	void DiagMethod();
}
#endif

#ifndef SERVER
class ClientOnly
{
}
#else
class ServerOnly
{
}
#endif

class Mixed extends Base
{
	void Always();
#ifdef DIAG_DEVELOPER
	void DiagHelper();
#endif
#ifndef SERVER
	void ClientHelper();
#else
	void ServerHelper();
#endif
	void Twice(int a);
}

class Dup
{
#ifdef DIAG_DEVELOPER
	void Twice(int a);
#endif
	void Twice(int a);
}

#ifdef DIAG_DEVELOPER
class DupClass extends Other
{
	void InactiveOnly();
}
#endif

class DupClass extends Base
{
	void ActiveOnly();
}

class DupClass2 extends Base
{
	void ActiveFirst();
}

#ifdef DIAG_DEVELOPER
class DupClass2 extends Other
{
	void InactiveSecond();
}
#endif

enum CondEnum
{
	A,
#ifdef DIAG_DEVELOPER
	B,
#endif
	C
}

#ifdef DIAG_DEVELOPER
enum DiagEnum
{
	X,
	Y
}
#endif

#ifdef DIAG_DEVELOPER
void Unbalanced() {
#else
void Balanced() {
#endif
	string s = "}";
}

void ParamSplit(int a
#ifdef DIAG_DEVELOPER
	, int b
#endif
);

#if DIAG_DEVELOPER
class ElifA {}
#elif SERVER
class ElifB {}
#elif defined(PLATFORM_LINUX)
class ElifC {}
#else
class ElifD {}
#endif

#if defined(SERVER) && !DIAG_DEVELOPER
class Complex {}
#endif
