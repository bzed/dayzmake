modded class Base
{
	int m_Added;
	override void Run(int x = 5, string s = "x,y", array<int> list = null);
	override void Run2();
	protected override void Protected();
	private override void Secret(int a);
	void AddedMethod();
	static void StaticAdded();
}

modded class OnlyModded
{
	override void Anything();
}

modded class Variant
{
}

modded enum AIStatus
{
	EXTRA
}

modded enum Counted
{
	D
}

enum Plain2
{
	R
}

sealed class Sealed extends Base
{
}
