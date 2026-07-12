def test_match(x):
    match x:
        case 1:
            return 'one'
        case _:
            return 'other'
print('match:', test_match(1))
print('DONE')
