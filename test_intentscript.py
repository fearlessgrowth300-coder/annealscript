"""Smallest runnable check: intent mapping heals renamed/missing/mistyped
fields instead of crashing. Run: python test_annealscript.py"""
from interpreter import run_source, StructInstance

SRC = """
struct User {
  username: string
  age: int
  email: string
}

let raw = {"usr_nm": "alice", "Age": "30", "email_address": "a@b.com"}
intent raw -> User as user

let broken = {"usr_nm": "bob"}
intent broken -> User as user2
"""


def demo():
    interp = run_source(SRC)

    user = interp.env["user"]
    assert isinstance(user, StructInstance)
    assert user.values["username"] == "alice"
    assert user.values["age"] == 30 and isinstance(user.values["age"], int)
    assert user.values["email"] == "a@b.com"

    user2 = interp.env["user2"]
    assert user2.values["username"] == "bob"
    assert user2.values["age"] is None
    assert user2.values["email"] is None

    print("ok")


BOUND_SRC = """
let speed = 0
let sensor_suggestion = 22

bound speed <= 15 {
  set speed = sensor_suggestion
}

let depth = 10
let obstacle_reading = 1

bound depth >= 2 {
  set depth = obstacle_reading
}
"""


def demo_bound():
    interp = run_source(BOUND_SRC)
    assert interp.env["speed"] == 15, "sensor pushed speed over the <=15 bound"
    assert interp.env["depth"] == 2, "sensor pushed depth under the >=2 bound"
    print("ok")


if __name__ == "__main__":
    demo()
    demo_bound()
