/*!
 * Copyright (c) Fixture Corp. All rights reserved.
 * License: synthetic test data.
 */

//! Plain typedefs and helpers of the fixture core.
typedef array<string> TStringArray;
typedef map<string, int> TStringIntMap; //! stays unused
//! Documented alias.
typedef array<ref Param> TParamArray;

const int FIXTURE_FLAG = 3;
int g_Counter;
static ref array<int> g_List = {1, 2};
int g_Table[] = {1,2};
string g_Url = "http://example.com/a;{b}";
protected autoptr Param g_Hidden;

/*! Adds two numbers. Second sentence is dropped. */
int Add(int a, int b)
{
	string s = "}{;//";
	return a + b;
}

proto native void FixtureNative(string text, out int result);
proto string FixtureString(notnull Param p);
private void HiddenFn() {}
[Obsolete("use Add")]
void OldAdd(int a = 1, int b = 2);

class Param<Class T>
{
	T m_Value;
	//! Gets the value.
	T Get();
	void Set(T v);
}

class Pair<Class K, typename V> extends Param<K>
{
	ref map<K, V> m_Map;
	static const string SEP = "a;b{c//d";
	void Put(K key, V value);
}

class Fwd;
class Empty {};
class Legacy : Param
{
}

//! Struct docs.
struct Vec2
{
	float x;
	float y;
}
