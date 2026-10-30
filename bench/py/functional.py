# 高阶函数：map / filter / sort / reduce 与闭包调用
from functools import reduce

def run(n):
    xs = list(map(lambda i: i * 7919 % 10007, range(n)))
    evens = list(filter(lambda x: x % 2 == 0, xs))
    s = sorted(evens)
    total = reduce(lambda acc, x: acc + x, s, 0)
    print(len(evens), s[0], s[-1], total)

run(3000000)
