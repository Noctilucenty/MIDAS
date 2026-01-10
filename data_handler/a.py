import requests
import json
def symbol():
    url2 = 'https://fapi.binance.com/fapi/v1/exchangeInfo'

    response2 = requests.get(url2)
    data2 = json.loads(response2.text)

    symbols2 = data2['symbols']
    spot2 = []
    for symbol in symbols2:
        if symbol['quoteAsset'] == 'USDT':
            if symbol['contractType'] == "PERPETUAL":
                if symbol['status'] == "TRADING":
                    if symbol['symbol'] == 'BTCUSDT':
                        spot2.append(symbol)

    print(spot2)

symbol()
