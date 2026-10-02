import axios from 'axios';
import { getApi } from '../axios-wrapper';
import { ApiBaseConfig } from '../types';
import { Transaction } from './types';

export const getTransactionByExternalIdForRelayer = async (
  relayerId: string,
  externalId: string,
  baseConfig: ApiBaseConfig
): Promise<Transaction | null> => {
  try {
    const response = await getApi<Transaction>(
      baseConfig,
      `transactions/relayers/${relayerId}/external/${externalId}`
    );
    return response.data;
  } catch (error) {
    if (axios.isAxiosError(error) && error.response?.status === 404) {
      return null;
    }
    throw error;
  }
};

export const getTransactionByExternalId = async (
  externalId: string,
  baseConfig: ApiBaseConfig
): Promise<Transaction | null> => {
  try {
    const response = await getApi<Transaction | null>(
      baseConfig,
      `transactions/external/${externalId}`
    );
    return response.data;
  } catch (error) {
    console.error('Failed to get transaction by externalId:', error);
    throw error;
  }
};
