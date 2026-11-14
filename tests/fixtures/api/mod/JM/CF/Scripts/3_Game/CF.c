#define CF_FIXTURE_LOCAL
  #define CF_INDENTED 1
//! The CF base.
class CFBase
{
	void Init();
}

#ifdef CF_FIXTURE
class CFSwitch extends CFBase
#else
class CFSwitch extends Base
#endif
{
}

#ifdef CF_FIXTURE_LOCAL
class LocalDefined {}
#endif

#ifdef CF_LATER
class TooEarly {}
#endif
#define CF_LATER
