THRESHOLD = 0.75


class Filter:
    def keep(self, score):
        return score >= THRESHOLD


print(Filter().keep(0.9))
