# 字符串：插值、拼接、分割、查找
def run(n):
    parts = []
    for i in range(n):
        parts.append(f"item{i}")
    text = ",".join(parts)
    back = text.split(",")
    total = 0
    sevens = 0
    for s in back:
        total += len(s)
        if s.endswith("7"):
            sevens += 1
    acc = ""
    for i in range(20000):
        acc = acc + "x"
    print(len(text), total, sevens, len(acc))
    print(text.upper()[0:16])

run(2000000)
