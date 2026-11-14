/** @brief The base of everything. It has more to say.
 *  @param nothing */
class Base
{
	/*! Counter, with a doc. Not this. */
	int m_Count;
	protected int m_Prot;
	private int m_Secret;
	static const int LIMIT = 10;
	static int s_Value = 5;
	int m_Arr[4];
	ref array<ref Base> m_Children;
	autoptr Param<int> m_Param;
	string m_Names[] = {"a;", "{b}", "//c"};
	int apple;
	int Banana;

	void Base();
	void ~Base();
	//! Run it.
	void Run(int x = 5, string s = "x,y", array<int> list = null);
	static Base GetInstance();
	protected void Protected();
	private void Secret(int a) { m_Secret = a; }
	proto native int Length();
	proto native owned string Text(notnull Base other, out string o, inout int io);
	proto external void External();
	[Obsolete]
	int OldMethod();
	[Attr("x"), Other]
	array<string> GetNames(int count = 1);
	int Inline() { return 1; }
	string Quote() { return "}{;//"; }
	event void OnEvent();
	override void Run2();
	string m_Doc; //!< not a doc
	bool Flag();
	bool flag();
	int a;
	int A;
}

#ifdef FEATURE_NETWORK_RECONCILIATION
class Variant extends Base
#else
class Variant extends Other
#endif
{
	void VariantMethod();
}

#ifndef FEATURE_NETWORK_RECONCILIATION
class Variant2 extends Base
#else
class Variant2 extends Other
#endif
{
}

#ifdef SOME_INACTIVE_DEFINE
class Variant3 extends Base
#else
class Variant3 extends Other
#endif
{
	int x;
}

/*! Foo.c */
class Other
{
}

//! @{
class AtDoc
{
}

//! Zeta documented.
class Zeta extends Base {}
class alpha extends Base {}
