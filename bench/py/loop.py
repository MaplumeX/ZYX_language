# 循环与整数运算
def run(n):
    total = 0
    i = 0
    while i < n:
        total += i % 7 * 3 - 1
        i += 1
    return total

print(run(30000000))
