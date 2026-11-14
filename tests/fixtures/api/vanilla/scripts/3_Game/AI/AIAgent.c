//! Statuses of an agent.
enum AIStatus
{
	//! Not started
	IDLE,
	RUNNING,
	//! Explicit value
	DONE = 10,
	AFTER,
	HEX = 0x10,
	NEXT,
	NEG = -3,
	NEGNEXT,
	PLUS = +5,
	BIN = 0b11,
	LEADING = 010,
	LOST,
	EXPR = 1 << 3,
	ALSOLOST,
	UNDER = 1_0,
	LAST
}

enum Counted
{
	A,
	B = 4,
	C
}

enum Plain : int
{
	A,
	B = 4,
	C
}

enum Plain2
{
	P = 0x7F,
	Q
}

enum Empty2
{
}

class AIAgent extends Base
{
	AIStatus GetStatus();
}
