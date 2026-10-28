import type { components } from '../generated/schema';

type S = components['schemas'];
export type Survey = S['Survey'];
export type SurveyInput = S['SurveyInput'];
export type SurveyQuestion = S['SurveyQuestion'];
export type SurveyQuestionKind = S['SurveyQuestionKind'];
export type SurveyStatus = S['SurveyStatus'];
export type SurveyState = S['SurveyState'];
export type SurveyForm = S['SurveyForm'];
export type SurveyResponseInput = S['SurveyResponseInput'];
export type SurveyResults = S['SurveyResults'];
export type SurveyQuestionResult = S['SurveyQuestionResult'];
export type SurveyOptionCount = S['SurveyOptionCount'];
export type SurveyNps = S['SurveyNps'];
export type SurveyTeamRate = S['SurveyTeamRate'];
export type SurveyResponseRow = S['SurveyResponseRow'];
/** Answer per question id: 1–5, 0–10, an option, options or text. */
export type SurveyAnswers = Record<string, unknown>;
