import pandas as pd
import h5py
import numpy as np
import os


pd.set_option('display.max_rows', None) 
pd.set_option('display.max_columns', None)  
pd.set_option('display.width', 1000)  
pd.set_option('display.max_colwidth', None)  

path = "C:\\Users\\workstation\\Desktop\\goldi_locks\\st_tester\\dataset\\Backtest_Market_data.h5"
dataset_name = 'interval_1m/ARKUSDT'

columns=['Open Time', 'Open', 'High', 'Low', 'Close', 'Volume', 'Quote Asset Volume', 'Number of Trades', 'Taker Buy Base Asset Volume', 'Taker Buy Quote Asset Volume']
pd.set_option('display.float_format', '{:.6f}'.format)

with h5py.File(path, 'r') as f:
    if dataset_name in f:
        data = f[dataset_name][:] #[:]
        print(pd.to_datetime(data[-30:][0]))
        print(data[-30:][3])
    #     processed_data = []
    #     for row in data:
    #         processed_row = []
    #         for value in row:
    #             processed_row.append(value)
    #         processed_data.append(processed_row)

    #     # processed_row = [value for value in data]

    #     processed_data = np.array(processed_data)
    #     df = pd.DataFrame(processed_data, columns=columns)

    #     # df = pd.DataFrame([processed_row], columns=columns)
    #     # df['Open Time'] = pd.to_datetime(df['Open Time'])
    #     print(df)  
    # else:
    #     print(f"{dataset_name} 데이터셋이 존재하지 않습니다.")
