import requests
import pandas as pd
import os
import time
from datetime import datetime, timedelta
import h5py
from datetime import datetime, timezone
import time
import os
import json


def symbol():
    url2 = 'https://fapi.binance.com/fapi/v1/exchangeInfo'

    response2 = requests.get(url2)
    data2 = json.loads(response2.text)

    symbols2 = data2['symbols'][:]
    spot2 = []
    for symbol in symbols2:
        if symbol['quoteAsset'] == 'USDT':
            spot2.append(symbol['symbol'])

    time.sleep(10)
    return spot2

pd.set_option('display.max_rows', None) 
pd.set_option('display.max_columns', None)  
pd.set_option('display.width', 1000)  
pd.set_option('display.max_colwidth', None)  

symbol_l = symbol()

# 바이낸스 API 엔드포인트
BASE_URL = 'https://fapi.binance.com/fapi/v1/klines'

path_p = 'C:\\Users\\workstation\\Desktop\\Backtest_Market_data.h5'

# 데이터 설정
interval = '1m'
all_data = pd.DataFrame()
columns=['Open Time', 'Open', 'High', 'Low', 'Close', 'Volume', 'Close Time', 'Quote Asset Volume', 'Number of Trades', 'Taker Buy Base Asset Volume', 'Taker Buy Quote Asset Volume', 'Ignore']



# for sym in symbol_l:
#     if not sym == 'BTCSTUSDT':

sym = "ETHUSDT"

print(sym)
dataset_name = f'interval_1m/{sym}'

if os.path.exists(path_p):
    with h5py.File(path_p, 'r') as f:
        data_set_searcher = f['interval_1m']
        if sym in data_set_searcher:
            print("그룹 있음")
            all_data_pre = f[dataset_name][:]
            last_Day = all_data_pre[-1][0]
            last_day = last_Day / (1000 * 1e6)
            dt = datetime.fromtimestamp(last_day)
            dt_plus_one_minute = dt + timedelta(minutes=1)
            start_date = dt_plus_one_minute.replace(tzinfo=timezone(timedelta(hours=9)))

            # 실제 데이터 열 수에 맞춰 열 이름 설정
            columns_pre = ['Open_Time', 'Open', 'High', 'Low', 'Close', 'Volume', 
                                            'Quote_Asset_Volume', 'Number_of_Trades', 
                                            'Taker_Buy_Base_Asset_Volume', 'Taker_Buy_Quote_Asset_Volume', 
                                            ]
            
            # 데이터프레임으로 변환
            df_pre = pd.DataFrame(all_data_pre, columns=columns_pre[:all_data_pre.shape[1]])

            # Open_Time 중복 확인
            duplicates_pre = df_pre[df_pre['Open_Time'].duplicated(keep=False)]  # 중복된 모든 행 찾기

            if not duplicates_pre.empty:
                print(f"중복된 Open_Time이 있는 행:\n{duplicates_pre}")
            else:
                print("중복된 Open_Time이 없습니다.")

        else:
            print("그룹 없음")
            start_date = datetime(2019, 12, 1, 0, 0).replace(tzinfo=timezone(timedelta(hours=9)))
else:
    print("파일 없음")
    start_date = datetime(2019, 12, 1, 0, 0).replace(tzinfo=timezone(timedelta(hours=9)))

start_c_date = start_date.astimezone(timezone.utc)
end_date = start_date + timedelta(minutes=1000)
end_loop_date = datetime(2025, 2, 12, 9, 0, 0, tzinfo=timezone.utc)


while True:
    try:
        if start_c_date > end_loop_date:
            break

        for i in range(300):
            
            df = pd.DataFrame()

            start_datetime = start_date.astimezone(timezone.utc)
            end_datetime = end_date.astimezone(timezone.utc)

            if start_datetime > end_loop_date:
                start_c_date = start_datetime
                break

            start_time = int(start_datetime.timestamp() * 1000)
            end_time = int(end_datetime.timestamp() * 1000)

            params = {
                'symbol': sym,
                'interval': interval,          
                'startTime': start_time,
                'endTime': end_time,
                'limit': 1000
            }

            response = requests.get(BASE_URL, params=params)
            
            if response.json():
                if response.status_code == 200:
                    ohlcv_data = response.json()
                    df = pd.DataFrame(ohlcv_data, columns=columns)
                    df.columns = df.columns.str.replace(' ', '_')
                    df.drop(columns=['Close_Time', 'Ignore'], inplace=True)
                    df['Open_Time'] = df['Open_Time']/1000
                    df['Open_Time'] = pd.to_datetime(df['Open_Time'], unit='s', utc=True)
                    df['Open_Time'] = df['Open_Time'].dt.tz_convert('Asia/Seoul')
                    try:
                        new_data = df.to_numpy(dtype='float64')
                        with h5py.File(path_p, 'a') as f:
                            if dataset_name in f:
                                dset = f[dataset_name]
                                current_shape = dset.shape[0]
                                dset.resize((current_shape + new_data.shape[0], new_data.shape[1]))
                                dset[current_shape:] = new_data
                                print(f"{sym} 저장 완료")
                            else:
                                f.create_dataset(dataset_name,data=new_data,maxshape=(None, new_data.shape[1]),chunks=(1440, new_data.shape[1]))
                                print(f"{sym} 저장 완료")
                            
                    except Exception as e:
                        print(e)
                        os._exit(0)
                else:
                    print(f"Error: {response.status_code}, {response.text}")
            else:
                pass

            start_date += timedelta(minutes=1000)
            end_date += timedelta(minutes=1000)
        
            time.sleep(0.1)
        
        time.sleep(61)
    except Exception as e:
        if 'Error: 400, {"code":-1122,"msg":"Invalid symbol status."}' in e:
            print(f"{sym} 없는 심볼")
            break